use std::slice;

use scene::Command;
use vello_cpu::kurbo::{Affine, Rect, RoundedRect, Shape};
use vello_cpu::peniko::Color;
use vello_cpu::{Glyph, Image, ImageSource, Pixmap, RenderContext, Resources};

fn color_of(v: u32) -> Color {
    Color::from_rgba8((v >> 24) as u8, (v >> 16) as u8, (v >> 8) as u8, v as u8)
}

fn draw_text(
    ctx: &mut RenderContext,
    resources: &mut Resources,
    text: &str,
    x: f64,
    y: f64,
    size: f32,
    color: Color,
    transform: Affine,
) {
    for run in scene::shape(text, size) {
        let glyphs = run.glyphs.iter().map(|g| Glyph {
            id: g.id,
            x: g.x,
            y: g.y,
        });
        ctx.set_paint(color);
        ctx.set_transform(transform * Affine::translate((x, y)));
        let _ = ctx
            .glyph_run(resources, &run.font)
            .font_size(run.size)
            .fill_glyphs(glyphs);
    }
    ctx.set_transform(transform);
}

#[no_mangle]
pub extern "C" fn cpu_render(w: i32, h: i32, list: *const u8, len: i32, out: *mut u8) -> i32 {
    if w <= 0 || h <= 0 || w > u16::MAX as i32 || h > u16::MAX as i32 {
        return -1;
    }
    if list.is_null() || out.is_null() || len < 0 {
        return -1;
    }
    let bytes = unsafe { slice::from_raw_parts(list, len as usize) };
    let commands = match scene::parse(bytes) {
        Ok(commands) => commands,
        Err(code) => return code,
    };
    let width = w as u16;
    let height = h as u16;
    let mut ctx = RenderContext::new(width, height);
    let mut resources = Resources::new();
    let mut transform = Affine::IDENTITY;
    let mut stack: Vec<Affine> = Vec::new();
    ctx.set_transform(transform);
    for command in &commands {
        match command {
            Command::Fill { color, x, y, w, h } => {
                ctx.set_transform(transform);
                ctx.set_paint(color_of(*color));
                ctx.fill_rect(&Rect::new(*x, *y, x + w, y + h));
            }
            Command::RoundRect {
                color,
                x,
                y,
                w,
                h,
                radius,
            } => {
                ctx.set_transform(transform);
                ctx.set_paint(color_of(*color));
                let path = RoundedRect::new(*x, *y, x + w, y + h, *radius).to_path(0.1);
                ctx.fill_path(&path);
            }
            Command::Text {
                color,
                x,
                y,
                size,
                text,
            } => draw_text(
                &mut ctx,
                &mut resources,
                text,
                *x,
                *y,
                *size,
                color_of(*color),
                transform,
            ),
            Command::Image { x, y, w, h, png } => {
                if let Some(image) = scene::decode_png(png) {
                    let iw = image.width as f64;
                    let ih = image.height as f64;
                    let source = ImageSource::from_peniko_image_data(&image);
                    ctx.set_transform(Affine::IDENTITY);
                    ctx.set_paint(Image {
                        image: source,
                        sampler: Default::default(),
                    });
                    ctx.set_transform(Affine::IDENTITY);
                    ctx.set_paint_transform(
                        transform
                            * Affine::translate((*x, *y))
                            * Affine::scale_non_uniform(w / iw, h / ih),
                    );
                    ctx.fill_rect(&Rect::new(*x, *y, x + w, y + h));
                    ctx.reset_paint_transform();
                }
            }
            Command::ClipPush { x, y, w, h } => {
                ctx.set_transform(transform);
                let path = Rect::new(*x, *y, x + w, y + h).to_path(0.1);
                ctx.push_clip_layer(&path);
            }
            Command::ClipPop => ctx.pop_layer(),
            Command::TransformPush(m) => {
                stack.push(transform);
                transform = transform * Affine::new(*m);
                ctx.set_transform(transform);
            }
            Command::TransformPop => {
                transform = stack.pop().unwrap_or(Affine::IDENTITY);
                ctx.set_transform(transform);
            }
        }
    }
    let mut pixmap = Pixmap::new(width, height);
    ctx.flush();
    ctx.render(&mut pixmap, &mut resources);

    let dst = unsafe { slice::from_raw_parts_mut(out, (w * h * 4) as usize) };
    for (i, pixel) in pixmap.data().iter().enumerate() {
        let a = pixel.a as u32;
        let at = i * 4;
        if a == 0 {
            dst[at] = 0;
            dst[at + 1] = 0;
            dst[at + 2] = 0;
            dst[at + 3] = 0;
        } else {
            dst[at] = ((pixel.r as u32 * 255 + a / 2) / a).min(255) as u8;
            dst[at + 1] = ((pixel.g as u32 * 255 + a / 2) / a).min(255) as u8;
            dst[at + 2] = ((pixel.b as u32 * 255 + a / 2) / a).min(255) as u8;
            dst[at + 3] = pixel.a;
        }
    }
    0
}

#[no_mangle]
pub extern "C" fn cpu_abi_version() -> i32 {
    1
}
