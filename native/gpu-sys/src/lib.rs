use std::ffi::c_void;

struct Gpu {
    instance: wgpu::Instance,
}

fn rgba(w: u32, h: u32, r: u8, g: u8, b: u8, a: u8, out: *mut u8) -> i32 {
    if out.is_null() || w == 0 || h == 0 {
        return -1;
    }
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let Some(adapter) =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()
    else {
        return -2;
    };
    let Ok((device, queue)) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
    else {
        return -3;
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
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: r as f64 / 255.0,
                        g: g as f64 / 255.0,
                        b: b as f64 / 255.0,
                        a: a as f64 / 255.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    }

    let bytes_per_pixel = 4u32;
    let unpadded = w * bytes_per_pixel;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded = unpadded.div_ceil(align) * align;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (padded as u64) * (h as u64),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
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
        return -4;
    }
    if rx.recv().is_err() {
        return -5;
    }
    let data = slice.get_mapped_range();
    let dst = unsafe { std::slice::from_raw_parts_mut(out, (w * h * bytes_per_pixel) as usize) };
    for row in 0..h as usize {
        let src = &data[row * padded as usize..row * padded as usize + unpadded as usize];
        let at = row * unpadded as usize;
        dst[at..at + unpadded as usize].copy_from_slice(src);
    }
    drop(data);
    buffer.unmap();
    0
}

#[no_mangle]
pub extern "C" fn gpu_instance_create() -> *mut c_void {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    Box::into_raw(Box::new(Gpu { instance })) as *mut c_void
}

#[no_mangle]
pub extern "C" fn gpu_instance_destroy(p: *mut c_void) {
    if p.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(p as *mut Gpu));
    }
}

#[no_mangle]
pub extern "C" fn gpu_adapter_count(p: *mut c_void) -> i32 {
    if p.is_null() {
        return -1;
    }
    let gpu = unsafe { &*(p as *const Gpu) };
    gpu.instance.enumerate_adapters(wgpu::Backends::all()).len() as i32
}

#[no_mangle]
pub extern "C" fn gpu_adapter_name(p: *mut c_void, index: i32, out: *mut u8, len: i32) -> i32 {
    if p.is_null() || out.is_null() || len <= 0 || index < 0 {
        return -1;
    }
    let gpu = unsafe { &*(p as *const Gpu) };
    let adapters = gpu.instance.enumerate_adapters(wgpu::Backends::all());
    let Some(adapter) = adapters.get(index as usize) else {
        return -2;
    };
    let name = adapter.get_info().name;
    let bytes = name.as_bytes();
    let n = bytes.len().min(len as usize - 1);
    let dst = unsafe { std::slice::from_raw_parts_mut(out, n) };
    dst.copy_from_slice(&bytes[..n]);
    unsafe {
        *out.add(n) = 0;
    }
    n as i32
}

#[no_mangle]
pub extern "C" fn gpu_clear_rgba(
    w: i32,
    h: i32,
    r: u8,
    g: u8,
    b: u8,
    a: u8,
    out: *mut u8,
) -> i32 {
    if w <= 0 || h <= 0 {
        return -1;
    }
    rgba(w as u32, h as u32, r, g, b, a, out)
}
