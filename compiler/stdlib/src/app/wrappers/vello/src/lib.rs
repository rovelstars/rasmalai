use std::slice;
use vello::kurbo::{Affine, Rect};
use vello::peniko::{Color, Fill};
use vello::{AaConfig, RenderParams, Renderer, RendererOptions};

fn open() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        {
            Ok(adapter) => adapter,
            Err(_) => pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                force_fallback_adapter: true,
                ..Default::default()
            }))
            .ok()?,
        };
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
}

fn readback(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    w: u32,
    h: u32,
    out: *mut u8,
) -> i32 {
    let unpadded = w * 4;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded = unpadded.div_ceil(align) * align;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (padded as u64) * (h as u64),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));

    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |res| {
        let _ = tx.send(res);
    });
    if device.poll(wgpu::PollType::wait_indefinitely()).is_err() {
        return -7;
    }
    if rx.recv().is_err() {
        return -8;
    }
    let data = match slice.get_mapped_range() {
        Ok(d) => d,
        Err(_) => return -9,
    };
    let dst = unsafe { slice::from_raw_parts_mut(out, (w * h * 4) as usize) };
    for row in 0..h as usize {
        let src = &data[row * padded as usize..row * padded as usize + unpadded as usize];
        let at = row * unpadded as usize;
        dst[at..at + unpadded as usize].copy_from_slice(src);
    }
    drop(data);
    buffer.unmap();
    0
}

fn render_scene(w: u32, h: u32, scene: &vello::Scene, out: *mut u8) -> i32 {
    let Some((device, queue)) = open() else {
        return -2;
    };
    let Ok(mut renderer) = Renderer::new(&device, RendererOptions::default()) else {
        return -4;
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let params = RenderParams {
        base_color: Color::TRANSPARENT,
        width: w,
        height: h,
        antialiasing_method: AaConfig::Area,
    };
    if renderer
        .render_to_texture(&device, &queue, scene, &view, &params)
        .is_err()
    {
        return -5;
    }
    readback(&device, &queue, &texture, w, h, out)
}

#[no_mangle]
pub extern "C" fn vello_fill(w: i32, h: i32, r: u8, g: u8, b: u8, a: u8, out: *mut u8) -> i32 {
    if w <= 0 || h <= 0 || out.is_null() {
        return -1;
    }
    let mut scene = vello::Scene::new();
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        Color::from_rgba8(r, g, b, a),
        None,
        &Rect::new(0.0, 0.0, w as f64, h as f64),
    );
    render_scene(w as u32, h as u32, &scene, out)
}

#[no_mangle]
pub extern "C" fn vello_render(w: i32, h: i32, list: *const u8, len: i32, out: *mut u8) -> i32 {
    if w <= 0 || h <= 0 || list.is_null() || out.is_null() || len < 0 {
        return -1;
    }
    let bytes = unsafe { slice::from_raw_parts(list, len as usize) };
    let scene = match scene::from_list(bytes) {
        Ok(scene) => scene,
        Err(code) => return code,
    };
    render_scene(w as u32, h as u32, &scene, out)
}

#[no_mangle]
pub extern "C" fn vello_abi_version() -> i32 {
    2
}
