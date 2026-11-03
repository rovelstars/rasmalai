use std::ffi::c_void;

struct Gpu {
    instance: wgpu::Instance,
}

const SHADER: &str = r#"
@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var pos = array<vec2<f32>, 3>(
        vec2<f32>(-0.6, -0.6),
        vec2<f32>(0.6, -0.6),
        vec2<f32>(0.0, 0.6),
    );
    return vec4<f32>(pos[i], 0.0, 1.0);
}

@fragment
fn fs() -> @location(0) vec4<f32> {
    return vec4<f32>(51.0 / 255.0, 204.0 / 255.0, 102.0 / 255.0, 1.0);
}
"#;

fn open() -> Option<(wgpu::Instance, wgpu::Device, wgpu::Queue)> {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::VULKAN;
    let instance = wgpu::Instance::new(descriptor);
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
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
    Some((instance, device, queue))
}

fn target(device: &wgpu::Device, w: u32, h: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
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
    })
}

fn color(r: u8, g: u8, b: u8, a: u8) -> wgpu::Color {
    wgpu::Color {
        r: r as f64 / 255.0,
        g: g as f64 / 255.0,
        b: b as f64 / 255.0,
        a: a as f64 / 255.0,
    }
}

fn pass(
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    clear: wgpu::Color,
    pipeline: Option<&wgpu::RenderPipeline>,
) {
    let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: None,
        multiview_mask: None,
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(clear),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
    });
    if let Some(p) = pipeline {
        rp.set_pipeline(p);
        rp.draw(0..3, 0..1);
    }
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
        return -4;
    }
    if rx.recv().is_err() {
        return -5;
    }
    let data = match slice.get_mapped_range() {
        Ok(d) => d,
        Err(_) => return -5,
    };
    let dst = unsafe { std::slice::from_raw_parts_mut(out, (w * h * 4) as usize) };
    for row in 0..h as usize {
        let src = &data[row * padded as usize..row * padded as usize + unpadded as usize];
        let at = row * unpadded as usize;
        dst[at..at + unpadded as usize].copy_from_slice(src);
    }
    drop(data);
    buffer.unmap();
    0
}

fn paint(w: u32, h: u32, clear: wgpu::Color, triangle: bool, out: *mut u8) -> i32 {
    if out.is_null() || w == 0 || h == 0 {
        return -1;
    }
    let Some((_instance, device, queue)) = open() else {
        return -2;
    };
    let texture = target(&device, w, h);
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let pipeline = if triangle {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        Some(
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            }),
        )
    } else {
        None
    };
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    pass(&mut encoder, &view, clear, pipeline.as_ref());
    queue.submit(Some(encoder.finish()));
    readback(&device, &queue, &texture, w, h, out)
}

#[no_mangle]
pub extern "C" fn wgpu_instance_create() -> *mut c_void {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::VULKAN;
    let instance = wgpu::Instance::new(descriptor);
    Box::into_raw(Box::new(Gpu { instance })) as *mut c_void
}

#[no_mangle]
pub extern "C" fn wgpu_instance_destroy(p: *mut c_void) {
    if p.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(p as *mut Gpu));
    }
}

#[no_mangle]
pub extern "C" fn wgpu_adapter_count(p: *mut c_void) -> i32 {
    if p.is_null() {
        return -1;
    }
    let gpu = unsafe { &*(p as *const Gpu) };
    pollster::block_on(gpu.instance.enumerate_adapters(wgpu::Backends::all())).len() as i32
}

#[no_mangle]
pub extern "C" fn wgpu_adapter_name(p: *mut c_void, index: i32, out: *mut u8, len: i32) -> i32 {
    if p.is_null() || out.is_null() || len <= 0 || index < 0 {
        return -1;
    }
    let gpu = unsafe { &*(p as *const Gpu) };
    let adapters = pollster::block_on(gpu.instance.enumerate_adapters(wgpu::Backends::all()));
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
pub extern "C" fn wgpu_clear_rgba(w: i32, h: i32, r: u8, g: u8, b: u8, a: u8, out: *mut u8) -> i32 {
    if w <= 0 || h <= 0 {
        return -1;
    }
    paint(w as u32, h as u32, color(r, g, b, a), false, out)
}

#[no_mangle]
pub extern "C" fn wgpu_triangle(w: i32, h: i32, out: *mut u8) -> i32 {
    if w <= 0 || h <= 0 {
        return -1;
    }
    paint(w as u32, h as u32, color(0, 0, 0, 255), true, out)
}

#[no_mangle]
pub extern "C" fn wgpu_abi_version() -> i32 {
    1
}
