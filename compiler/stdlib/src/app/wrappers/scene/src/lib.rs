use std::io::Cursor;
use std::sync::Arc;

use vello::kurbo::{Affine, Rect, RoundedRect};
use vello::peniko::{BlendMode, Blob, Color, Fill, ImageData, ImageFormat};
use vello::Glyph;

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
            _ => {}
        }
    }
    Ok(out)
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

fn draw_text(
    scene: &mut vello::Scene,
    text: &str,
    x: f64,
    y: f64,
    size: f32,
    color: Color,
    transform: Affine,
) {
    let mut font_cx = parley::FontContext::new();
    let mut layout_cx: parley::LayoutContext<()> = parley::LayoutContext::new();
    let mut builder = layout_cx.ranged_builder(&mut font_cx, text, 1.0, true);
    builder.push_default(parley::StyleProperty::FontSize(size));
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    for line in layout.lines() {
        for item in line.items() {
            if let parley::PositionedLayoutItem::GlyphRun(run) = item {
                let shaped = run.run();
                let glyphs = run.positioned_glyphs().map(|g| Glyph {
                    id: g.id,
                    x: g.x,
                    y: g.y,
                });
                scene
                    .draw_glyphs(shaped.font())
                    .font_size(shaped.font_size())
                    .brush(color)
                    .transform(transform * Affine::translate((x, y)))
                    .draw(Fill::NonZero, glyphs);
            }
        }
    }
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
            } => draw_text(&mut scene, text, *x, *y, *size, color_of(*color), transform),
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
