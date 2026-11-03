use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

use vello::kurbo::{Affine, BezPath, Rect, RoundedRect};
use vello::peniko::{BlendMode, Blob, Color, Fill, ImageData, ImageFormat};

const MAGIC: u32 = 0x4458_4E52;
const STRIDE: usize = 64;
const HEADER: usize = 32;

/// One decoded display-list command.
pub enum Command {
    Fill {
        color: u32,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    },
    RoundRect {
        color: u32,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        radius: f64,
    },
    Text {
        color: u32,
        x: f64,
        y: f64,
        size: f32,
        text: String,
    },
    Image {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        png: Vec<u8>,
    },
    ClipPush {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    },
    ClipPop,
    TransformPush([f64; 6]),
    TransformPop,
    Polygon {
        color: u32,
        points: Vec<(f64, f64)>,
    },
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    let chunk = bytes.get(at..at + 4)?;
    Some(u32::from_le_bytes(chunk.try_into().ok()?))
}

fn f64_at(bytes: &[u8], at: usize) -> Option<f64> {
    let chunk = bytes.get(at..at + 8)?;
    Some(f64::from_le_bytes(chunk.try_into().ok()?))
}

/// Decode display-list v2 into commands.
pub fn parse(bytes: &[u8]) -> Result<Vec<Command>, i32> {
    if bytes.len() < HEADER {
        return Err(-1);
    }
    if u32_at(bytes, 0) != Some(MAGIC) {
        return Err(-10);
    }
    if u32_at(bytes, 4) != Some(2) {
        return Err(-11);
    }
    let records = u32_at(bytes, 8).ok_or(-12)? as usize;
    if u32_at(bytes, 12).map(|v| v as usize) != Some(STRIDE) {
        return Err(-13);
    }
    let blob_at = u32_at(bytes, 16).ok_or(-14)? as usize;
    let blob_len = u32_at(bytes, 20).ok_or(-15)? as usize;
    let body_end = HEADER
        .checked_add(records.saturating_mul(STRIDE))
        .ok_or(-16)?;
    let blob_end = blob_at.checked_add(blob_len).ok_or(-16)?;
    if body_end > bytes.len() || blob_end > bytes.len() || blob_at < body_end {
        return Err(-17);
    }
    let blob = &bytes[blob_at..blob_end];

    let mut out = Vec::with_capacity(records);
    for i in 0..records {
        let at = HEADER + i * STRIDE;
        let kind = u32_at(bytes, at).ok_or(-19)?;
        let color = u32_at(bytes, at + 4).unwrap_or(0);
        let a = u32_at(bytes, at + 8).unwrap_or(0) as usize;
        let b = u32_at(bytes, at + 12).unwrap_or(0) as usize;
        let x = f64_at(bytes, at + 16).ok_or(-19)?;
        let y = f64_at(bytes, at + 24).ok_or(-19)?;
        let w = f64_at(bytes, at + 32).ok_or(-19)?;
        let h = f64_at(bytes, at + 40).ok_or(-19)?;
        let e = f64_at(bytes, at + 48).ok_or(-19)?;
        let f = f64_at(bytes, at + 56).ok_or(-19)?;
        match kind {
            1 => out.push(Command::Fill { color, x, y, w, h }),
            2 => {
                let end = a.checked_add(b).ok_or(-20)?;
                let text = blob.get(a..end).ok_or(-20)?;
                let text = std::str::from_utf8(text).map_err(|_| -18)?.to_string();
                out.push(Command::Text {
                    color,
                    x,
                    y,
                    size: w as f32,
                    text,
                });
            }
            3 => out.push(Command::ClipPush { x, y, w, h }),
            4 => out.push(Command::ClipPop),
            5 => out.push(Command::RoundRect {
                color,
                x,
                y,
                w,
                h,
                radius: e,
            }),
            6 => {
                let end = a.checked_add(b).ok_or(-20)?;
                let png = blob.get(a..end).ok_or(-20)?.to_vec();
                out.push(Command::Image { x, y, w, h, png });
            }
            7 => out.push(Command::TransformPush([x, y, w, h, e, f])),
            8 => out.push(Command::TransformPop),
            9 => {
                let mut points = Vec::with_capacity(b);
                for p in 0..b {
                    let at = a + p * 16;
                    let px = f64_at(blob, at).ok_or(-20)?;
                    let py = f64_at(blob, at + 8).ok_or(-20)?;
                    points.push((px, py));
                }
                out.push(Command::Polygon { color, points });
            }
            _ => {}
        }
    }
    Ok(out)
}

fn poly_path(points: &[(f64, f64)]) -> Option<BezPath> {
    let (first, rest) = points.split_first()?;
    let mut path = BezPath::new();
    path.move_to(*first);
    for p in rest {
        path.line_to(*p);
    }
    path.close_path();
    Some(path)
}

fn color_of(v: u32) -> Color {
    Color::from_rgba8((v >> 24) as u8, (v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// Decode a PNG into straight RGBA8 image data.
pub fn decode_png(bytes: &[u8]) -> Option<ImageData> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(
        png::Transformations::EXPAND | png::Transformations::ALPHA | png::Transformations::STRIP_16,
    );
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    buf.truncate(info.buffer_size());
    Some(ImageData {
        data: Blob::new(Arc::new(buf)),
        format: ImageFormat::Rgba8,
        alpha_type: vello::peniko::ImageAlphaType::Alpha,
        width: info.width,
        height: info.height,
    })
}

/// One shaped glyph, independent of the renderer that consumes it.
#[derive(Clone, Copy)]
pub struct Glyph {
    pub id: u32,
    pub x: f32,
    pub y: f32,
}

/// One glyph run with its font and size.
#[derive(Clone)]
pub struct ShapedRun {
    pub font: parley::FontData,
    pub size: f32,
    pub glyphs: Vec<Glyph>,
}

thread_local! {
    static SHAPER: RefCell<Option<(parley::FontContext, parley::LayoutContext<()>)>> =
        const { RefCell::new(None) };
    static CACHE: RefCell<HashMap<(u32, String), Vec<ShapedRun>>> =
        RefCell::new(HashMap::new());
}

/// Shape text once and reuse the result. Fonts are discovered once per thread.
pub fn shape(text: &str, size: f32) -> Vec<ShapedRun> {
    let key = (size.to_bits(), text.to_string());
    if let Some(runs) = CACHE.with(|cache| cache.borrow().get(&key).cloned()) {
        return runs;
    }
    let runs = SHAPER.with(|slot| {
        let mut slot = slot.borrow_mut();
        let (font_cx, layout_cx) = slot.get_or_insert_with(|| {
            (
                parley::FontContext::new(),
                parley::LayoutContext::<()>::new(),
            )
        });
        let mut builder = layout_cx.ranged_builder(font_cx, text, 1.0, true);
        builder.push_default(parley::StyleProperty::FontSize(size));
        let mut layout = builder.build(text);
        layout.break_all_lines(None);
        let mut runs = Vec::new();
        for line in layout.lines() {
            for item in line.items() {
                if let parley::PositionedLayoutItem::GlyphRun(run) = item {
                    let shaped = run.run();
                    let glyphs = run
                        .positioned_glyphs()
                        .map(|g| Glyph {
                            id: g.id,
                            x: g.x,
                            y: g.y,
                        })
                        .collect();
                    runs.push(ShapedRun {
                        font: shaped.font().clone(),
                        size: shaped.font_size(),
                        glyphs,
                    });
                }
            }
        }
        runs
    });
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() > 256 {
            cache.clear();
        }
        cache.insert(key, runs.clone());
    });
    runs
}

/// Turn decoded commands into a GPU scene.
pub fn to_gpu(commands: &[Command]) -> vello::Scene {
    let mut scene = vello::Scene::new();
    let mut transform = Affine::IDENTITY;
    let mut stack: Vec<Affine> = Vec::new();
    for command in commands {
        match command {
            Command::Fill { color, x, y, w, h } => {
                scene.fill(
                    Fill::NonZero,
                    transform,
                    color_of(*color),
                    None,
                    &Rect::new(*x, *y, x + w, y + h),
                );
            }
            Command::RoundRect {
                color,
                x,
                y,
                w,
                h,
                radius,
            } => {
                scene.fill(
                    Fill::NonZero,
                    transform,
                    color_of(*color),
                    None,
                    &RoundedRect::new(*x, *y, x + w, y + h, *radius),
                );
            }
            Command::Text {
                color,
                x,
                y,
                size,
                text,
            } => {
                for run in shape(text, *size) {
                    let glyphs = run.glyphs.iter().map(|g| vello::Glyph {
                        id: g.id,
                        x: g.x,
                        y: g.y,
                    });
                    scene
                        .draw_glyphs(&run.font)
                        .font_size(run.size)
                        .brush(color_of(*color))
                        .transform(transform * Affine::translate((*x, *y)))
                        .draw(Fill::NonZero, glyphs);
                }
            }
            Command::Image { x, y, w, h, png } => {
                if let Some(image) = decode_png(png) {
                    let iw = image.width as f64;
                    let ih = image.height as f64;
                    let at = transform
                        * Affine::translate((*x, *y))
                        * Affine::scale_non_uniform(w / iw, h / ih);
                    scene.draw_image(&image, at);
                }
            }
            Command::ClipPush { x, y, w, h } => {
                scene.push_layer(
                    Fill::NonZero,
                    BlendMode::default(),
                    1.0,
                    transform,
                    &Rect::new(*x, *y, x + w, y + h),
                );
            }
            Command::Polygon { color, points } => {
                if let Some(path) = poly_path(points) {
                    scene.fill(
                        Fill::NonZero,
                        transform,
                        color_of(*color),
                        None,
                        &path,
                    );
                }
            }
            Command::ClipPop => scene.pop_layer(),
            Command::TransformPush(m) => {
                stack.push(transform);
                transform = transform * Affine::new(*m);
            }
            Command::TransformPop => {
                transform = stack.pop().unwrap_or(Affine::IDENTITY);
            }
        }
    }
    scene
}

/// Decode display-list v2 into a vello scene.
pub fn from_list(bytes: &[u8]) -> Result<vello::Scene, i32> {
    Ok(to_gpu(&parse(bytes)?))
}
