#[cfg(not(target_os = "linux"))]
compile_error!(
    "the Rasmalai UI host supports Linux Wayland only; see requirements_UI/72-platform-support.md"
);

mod surface_socket;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ffi::c_void;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::ptr::NonNull;
use std::slice;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use accesskit::{
    Action, ActionData, ActionHandler, ActionRequest, ActivationHandler, DeactivationHandler,
    Invalid, Live, Node, NodeId, Rect, Role, Toggled, Tree, TreeUpdate,
};
use accesskit_unix::Adapter;

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, RawDisplayHandle, RawWindowHandle,
    WaylandDisplayHandle, WaylandWindowHandle,
};
use vello::kurbo::{Affine, Rect as KurboRect};
use vello::peniko::{BlendMode, Blob, Color, Fill, ImageAlphaType, ImageData, ImageFormat};
use vello::{AaConfig, RenderParams, Renderer, RendererOptions};
use wayland_client::{
    delegate_noop,
    globals::{registry_queue_init, GlobalListContents},
    protocol::{
        wl_compositor, wl_data_device, wl_data_device_manager, wl_data_offer, wl_data_source,
        wl_callback, wl_keyboard, wl_pointer, wl_registry, wl_seat, wl_shm, wl_surface,
        wl_touch,
    },
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum,
};
use wayland_cursor::CursorTheme;
use wayland_protocols::wp::pointer_constraints::zv1::client::{
    zwp_confined_pointer_v1::ZwpConfinedPointerV1,
    zwp_locked_pointer_v1::ZwpLockedPointerV1,
    zwp_pointer_constraints_v1::{self, ZwpPointerConstraintsV1},
};
use wayland_protocols::wp::text_input::zv3::client::{
    zwp_text_input_manager_v3::ZwpTextInputManagerV3,
    zwp_text_input_v3::{self, ZwpTextInputV3},
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

const BTN_LEFT: u32 = 0x110;

enum Event {
    Close,
    Resize(i32, i32),
    PointerDown(i32, i32),
    PointerUp(i32, i32),
    PointerMove(i32, i32),
    KeyDown(i32),
    KeyUp(i32),
    SurfaceLost,
    Scroll(i32),
    PointerLeave,
    Text(Vec<u8>),
    TextMode,
    Clipboard,
    ImePreedit(Vec<u8>),
    ImeCommit(Vec<u8>),
    ImeDelete(i32, i32),
    ImeReset,
    GamepadAxis(i32, i32),
    GamepadButton(i32, i32),
    GamepadConnected,
    A11yAction(i32, i32, Vec<u8>),
    TouchDown(i32, i32, i32),
    TouchMove(i32, i32, i32),
    TouchUp(i32, i32, i32),
    TouchCancel(i32),
    Suspended,
    Resumed,
}

struct GpuCore {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Mutex<Renderer>,
    blur: Mutex<Option<BlurPass>>,
}

static CORE: OnceLock<Option<GpuCore>> = OnceLock::new();

#[derive(Debug)]
struct RawDisplay(RawDisplayHandle);

// The raw display is the live wl_display behind the Host connection.
unsafe impl Send for RawDisplay {}
unsafe impl Sync for RawDisplay {}

impl HasDisplayHandle for RawDisplay {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        Ok(unsafe { DisplayHandle::borrow_raw(self.0) })
    }
}

fn candidates(choice: i32) -> Vec<wgpu::Backends> {
    match choice {
        1 => vec![wgpu::Backends::VULKAN],
        2 => vec![wgpu::Backends::GL],
        _ => vec![wgpu::Backends::VULKAN, wgpu::Backends::GL],
    }
}

fn open_resources(
    choice: i32,
    handles: Option<(RawDisplayHandle, RawWindowHandle)>,
) -> Option<(GpuCore, Option<wgpu::Surface<'static>>)> {
    for backend in candidates(choice) {
        let mut descriptor = match handles {
            Some((display, _)) => {
                wgpu::InstanceDescriptor::new_with_display_handle(Box::new(RawDisplay(display)))
            }
            None => wgpu::InstanceDescriptor::new_without_display_handle(),
        };
        descriptor.backends = backend;
        let instance = wgpu::Instance::new(descriptor);
        let surface = handles.and_then(|(display, window)| {
            let created: Result<wgpu::Surface<'static>, _> = unsafe {
                instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: Some(display),
                    raw_window_handle: window,
                })
            };
            created.ok()
        });
        if handles.is_some() && surface.is_none() {
            continue;
        }
        let adapter_options = wgpu::RequestAdapterOptions {
            compatible_surface: surface.as_ref(),
            ..Default::default()
        };
        let adapter = match pollster::block_on(instance.request_adapter(&adapter_options)) {
            Ok(adapter) => adapter,
            Err(_) => {
                match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                    force_fallback_adapter: true,
                    ..adapter_options
                })) {
                    Ok(adapter) => adapter,
                    Err(_) => continue,
                }
            }
        };
        let Ok((device, queue)) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
        else {
            continue;
        };
        let Ok(renderer) = Renderer::new(
            &device,
            RendererOptions {
                antialiasing_support: vello::AaSupport::area_only(),
                ..Default::default()
            },
        ) else {
            continue;
        };
        return Some((
            GpuCore {
                instance,
                adapter,
                device,
                queue,
                renderer: Mutex::new(renderer),
                blur: Mutex::new(None),
            },
            surface,
        ));
    }
    None
}

fn core(choice: i32) -> Option<&'static GpuCore> {
    CORE.get_or_init(|| open_resources(choice, None).map(|(core, _)| core))
        .as_ref()
}

fn make_target(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    make_target_format(device, width, height, wgpu::TextureFormat::Rgba8Unorm)
}

fn make_target_format(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("host target"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

const BLUR_SHADER: &str = r#"
struct BlurUniforms {
    direction: vec2<f32>,
    sigma: f32,
    radius: f32,
};

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;
@group(0) @binding(2) var<uniform> uniforms: BlurUniforms;

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32) -> @builtin(position) vec4<f32> {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0)
    );
    return vec4<f32>(positions[vertex], 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let dimensions = textureDimensions(source);
    let size = vec2<f32>(f32(dimensions.x), f32(dimensions.y));
    let uv = position.xy / size;
    let sigma = max(uniforms.sigma, 0.0001);
    let radius = i32(uniforms.radius);
    let scale = 1.0 / (2.0 * sigma * sigma);
    var total = vec4<f32>(0.0);
    var weight_total = 0.0;
    for (var offset: i32 = -radius; offset <= radius; offset = offset + 1) {
        let x = f32(offset);
        let weight = exp(-x * x * scale);
        let sample_uv = uv + uniforms.direction * x / size;
        total = total + textureSample(source, source_sampler, sample_uv) * weight;
        weight_total = weight_total + weight;
    }
    return total / weight_total;
}
"#;

struct BlurPass {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
}

impl BlurPass {
    fn new(device: &wgpu::Device) -> Option<Self> {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blur shader"),
            source: wgpu::ShaderSource::Wgsl(BLUR_SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blur bind group layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("blur pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blur pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("blur sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        Some(Self {
            layout,
            pipeline,
            sampler,
        })
    }

    fn apply(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        source: &wgpu::TextureView,
        target: &wgpu::TextureView,
        direction: [f32; 2],
        sigma: f32,
        radius: u32,
    ) {
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("blur uniform"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut bytes = [0u8; 16];
        bytes[0..4].copy_from_slice(&direction[0].to_le_bytes());
        bytes[4..8].copy_from_slice(&direction[1].to_le_bytes());
        bytes[8..12].copy_from_slice(&sigma.to_le_bytes());
        bytes[12..16].copy_from_slice(&(radius as f32).to_le_bytes());
        queue.write_buffer(&uniform, 0, &bytes);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blur bind group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform.as_entire_binding(),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("blur encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("blur pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit(Some(encoder.finish()));
    }
}

fn blur_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    label: &'static str,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

fn unregister_images(core: &GpuCore, images: &[ImageData]) {
    if images.is_empty() {
        return;
    }
    if let Ok(mut renderer) = core.renderer.lock() {
        for image in images {
            renderer.override_image(image, None);
        }
    }
}

fn trace(message: &str) {
    if std::env::var_os("RNX_TRACE").is_some() {
        eprintln!("{message}");
    }
}

fn render_blur_body(
    core: &GpuCore,
    commands: &[scene::Command],
    spec: &scene::BlurSpec,
    width: u32,
    height: u32,
) -> Option<ImageData> {
    let body = commands.get(spec.start..spec.end)?;
    let nested = scene::collect_blurs(body, spec.transform);
    let mut nested_images = Vec::with_capacity(nested.len());
    for nested_spec in &nested {
        match render_blur_body(core, body, nested_spec, width, height) {
            Some(image) => nested_images.push(image),
            None => {
                unregister_images(core, &nested_images);
                return None;
            }
        }
    }

    let body_scene = scene::to_gpu_images(body, spec.transform, &nested_images);
    let mut clipped = vello::Scene::new();
    clipped.push_layer(
        Fill::NonZero,
        BlendMode::default(),
        1.0,
        spec.transform,
        &KurboRect::new(spec.x, spec.y, spec.x + spec.w, spec.y + spec.h),
    );
    clipped.append(&body_scene, None);
    clipped.pop_layer();

    let (source_texture, source_view) = blur_texture(&core.device, width, height, "blur source");
    let (target_texture, target_view) = blur_texture(&core.device, width, height, "blur target");
    let rendered = match core.renderer.lock() {
        Ok(mut renderer) => {
            let params = RenderParams {
                base_color: Color::TRANSPARENT,
                width: width.max(1),
                height: height.max(1),
                antialiasing_method: AaConfig::Area,
            };
            renderer
                .render_to_texture(
                    &core.device,
                    &core.queue,
                    &clipped,
                    &source_view,
                    &params,
                )
                .is_ok()
        }
        Err(_) => false,
    };
    unregister_images(core, &nested_images);
    if !rendered {
        trace("blur: body render failed, painting unblurred");
        return None;
    }

    let sigma = if spec.sigma.is_finite() {
        spec.sigma.max(0.0)
    } else {
        0.0
    };
    if sigma > 0.0 {
        let radius = (3.0 * sigma).ceil().clamp(1.0, 48.0) as u32;
        let mut slot = match core.blur.lock() {
            Ok(slot) => slot,
            Err(_) => return None,
        };
        if slot.is_none() {
            *slot = BlurPass::new(&core.device);
        }
        let Some(pass) = slot.as_ref() else {
            trace("blur: pipeline unavailable, painting unblurred");
            return None;
        };
        pass.apply(
            &core.device,
            &core.queue,
            &source_view,
            &target_view,
            [1.0, 0.0],
            sigma as f32,
            radius,
        );
        pass.apply(
            &core.device,
            &core.queue,
            &target_view,
            &source_view,
            [0.0, 1.0],
            sigma as f32,
            radius,
        );
    }

    let image = ImageData {
        data: Blob::new(Arc::new(&[])),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::AlphaPremultiplied,
        width: width.max(1),
        height: height.max(1),
    };
    let mut renderer = core.renderer.lock().ok()?;
    renderer.override_image(
        &image,
        Some(wgpu::TexelCopyTextureInfoBase {
            texture: source_texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        }),
    );
    drop(renderer);
    drop(target_texture);
    Some(image)
}

fn render_scene(
    core: &GpuCore,
    scene: &vello::Scene,
    target: &wgpu::TextureView,
    width: u32,
    height: u32,
) -> Result<(), i32> {
    let params = RenderParams {
        base_color: Color::TRANSPARENT,
        width: width.max(1),
        height: height.max(1),
        antialiasing_method: AaConfig::Area,
    };
    let mut renderer = core.renderer.lock().map_err(|_| -5)?;
    renderer
        .render_to_texture(&core.device, &core.queue, scene, target, &params)
        .map_err(|_| -5)
}

fn render_commands(
    core: &GpuCore,
    commands: &[scene::Command],
    target: &wgpu::TextureView,
    width: u32,
    height: u32,
    start: Affine,
) -> Result<(), i32> {
    let specs = scene::collect_blurs(commands, start);
    if specs.is_empty() {
        let scene = scene::to_gpu_transformed(commands, start);
        return render_scene(core, &scene, target, width, height);
    }

    let mut images = Vec::with_capacity(specs.len());
    for spec in &specs {
        match render_blur_body(core, commands, spec, width, height) {
            Some(image) => images.push(image),
            None => {
                unregister_images(core, &images);
                trace("blur: falling back to unblurred scene");
                let scene = scene::to_gpu_transformed(commands, start);
                return render_scene(core, &scene, target, width, height);
            }
        }
    }
    let scene = scene::to_gpu_images(commands, start, &images);
    let result = render_scene(core, &scene, target, width, height);
    unregister_images(core, &images);
    result
}

struct Gpu {
    resources: GpuCore,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    target: wgpu::Texture,
    target_view: wgpu::TextureView,
    scratch: Option<(wgpu::Texture, wgpu::TextureView, u32, u32)>,
}

struct A11yActivation(Arc<Mutex<Option<TreeUpdate>>>);

impl ActivationHandler for A11yActivation {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        self.0.lock().ok().and_then(|tree| tree.clone())
    }
}

struct A11yActions(mpsc::Sender<ActionRequest>);

impl ActionHandler for A11yActions {
    fn do_action(&mut self, request: ActionRequest) {
        let _ = self.0.send(request);
    }
}

struct A11yDeactivation;

impl DeactivationHandler for A11yDeactivation {
    fn deactivate_accessibility(&mut self) {}
}

const A11Y_MAGIC: u32 = 42394;
const A11Y_VERSION_V5: u32 = 5;
const A11Y_VERSION_V6: u32 = 6;
const A11Y_HEADER_V4: usize = 8;
const A11Y_HEADER_V5: usize = 16;
const A11Y_HEADER_V6: usize = 16;
const A11Y_STRIDE_V4: usize = 52;
const A11Y_STRIDE_V5: usize = 96;
const A11Y_STRIDE_V6: usize = 112;
const A11Y_V6_LABEL: usize = 96;
const A11Y_V6_DESCRIBED_BY: usize = 100;
const A11Y_V6_CONTROLS: usize = 104;
const A11Y_V6_ANNOUNCE: usize = 108;

#[derive(Clone, Debug)]
pub struct A11yRecord {
    id: i32,
    parent: i32,
    role: i32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    actions: u32,
    flags: u32,
    name: String,
    value: String,
    expanded: i32,
    invalid: u32,
    level: u32,
    live: u32,
    value_min: f64,
    value_max: f64,
    value_step: f64,
    description: String,
    label_id: i32,
    described_by_id: i32,
    controls_id: i32,
    announce_seq: u32,
}

fn read_u32(data: &[u8], off: usize) -> Option<u32> {
    let end = off.checked_add(4)?;
    if end > data.len() {
        return None;
    }
    Some(u32::from_le_bytes([
        data[off],
        data[off + 1],
        data[off + 2],
        data[off + 3],
    ]))
}

fn read_i32(data: &[u8], off: usize) -> Option<i32> {
    read_u32(data, off).map(|v| v as i32)
}

fn read_f64(data: &[u8], off: usize) -> Option<f64> {
    let end = off.checked_add(8)?;
    if end > data.len() {
        return None;
    }
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&data[off..end]);
    Some(f64::from_le_bytes(bytes))
}

fn read_string(data: &[u8], off: u32, len: u32) -> String {
    let start = off as usize;
    let Some(end) = start.checked_add(len as usize) else {
        return String::new();
    };
    data.get(start..end)
        .map(|b| String::from_utf8_lossy(b).into_owned())
        .unwrap_or_default()
}

fn parse_a11y(data: &[u8]) -> Vec<A11yRecord> {
    if read_u32(data, 0) != Some(A11Y_MAGIC) {
        return Vec::new();
    }
    // v5 and v6 headers both carry the stride at offset 12. A packet with the
    // v6 stride is parsed as v6 only, so an unknown version fails closed
    // instead of falling back to v4.
    if data.len() >= A11Y_HEADER_V6 {
        match read_u32(data, 12) {
            Some(stride) if stride as usize == A11Y_STRIDE_V6 => {
                return parse_a11y_v6(data).unwrap_or_default();
            }
            Some(stride) if stride as usize == A11Y_STRIDE_V5 => {
                return parse_a11y_v5(data).unwrap_or_default();
            }
            _ => {}
        }
    }
    parse_a11y_v4(data)
}

fn parse_record_v5_at(data: &[u8], o: usize) -> Option<A11yRecord> {
    let name_off = read_u32(data, o + 36)?;
    let name_len = read_u32(data, o + 40)?;
    let value_off = read_u32(data, o + 44)?;
    let value_len = read_u32(data, o + 48)?;
    let desc_len = read_u32(data, o + 92)?;
    Some(A11yRecord {
        id: read_i32(data, o)?,
        parent: read_i32(data, o + 4)?,
        role: read_i32(data, o + 8)?,
        x: read_i32(data, o + 12)?,
        y: read_i32(data, o + 16)?,
        w: read_i32(data, o + 20)?,
        h: read_i32(data, o + 24)?,
        actions: read_u32(data, o + 28)?,
        flags: read_u32(data, o + 32)?,
        name: read_string(data, name_off, name_len),
        value: read_string(data, value_off, value_len),
        expanded: read_i32(data, o + 52)?,
        invalid: read_u32(data, o + 56)?,
        level: read_u32(data, o + 60)?,
        live: read_u32(data, o + 64)?,
        value_min: read_f64(data, o + 68)?,
        value_max: read_f64(data, o + 76)?,
        value_step: read_f64(data, o + 84)?,
        description: read_string(data, value_off.saturating_add(value_len), desc_len),
        label_id: 0,
        described_by_id: 0,
        controls_id: 0,
        announce_seq: 0,
    })
}

fn parse_a11y_v5(data: &[u8]) -> Option<Vec<A11yRecord>> {
    if read_u32(data, 0)? != A11Y_MAGIC {
        return None;
    }
    if read_u32(data, 4)? != A11Y_VERSION_V5 {
        return None;
    }
    let count = read_u32(data, 8)? as usize;
    if read_u32(data, 12)? as usize != A11Y_STRIDE_V5 {
        return None;
    }
    let total = A11Y_HEADER_V5.checked_add(count.checked_mul(A11Y_STRIDE_V5)?)?;
    if total > data.len() {
        return None;
    }
    let mut out = Vec::with_capacity(count);
    let mut i = 0;
    while i < count {
        out.push(parse_record_v5_at(
            data,
            A11Y_HEADER_V5 + i * A11Y_STRIDE_V5,
        )?);
        i += 1;
    }
    Some(out)
}

fn parse_a11y_v6(data: &[u8]) -> Option<Vec<A11yRecord>> {
    if read_u32(data, 0)? != A11Y_MAGIC {
        return None;
    }
    if read_u32(data, 4)? != A11Y_VERSION_V6 {
        return None;
    }
    let count = read_u32(data, 8)? as usize;
    if read_u32(data, 12)? as usize != A11Y_STRIDE_V6 {
        return None;
    }
    let total = A11Y_HEADER_V6.checked_add(count.checked_mul(A11Y_STRIDE_V6)?)?;
    if total > data.len() {
        return None;
    }
    let mut out = Vec::with_capacity(count);
    let mut i = 0;
    while i < count {
        let o = A11Y_HEADER_V6 + i * A11Y_STRIDE_V6;
        let mut record = parse_record_v5_at(data, o)?;
        record.label_id = read_i32(data, o + A11Y_V6_LABEL)?;
        record.described_by_id = read_i32(data, o + A11Y_V6_DESCRIBED_BY)?;
        record.controls_id = read_i32(data, o + A11Y_V6_CONTROLS)?;
        record.announce_seq = read_u32(data, o + A11Y_V6_ANNOUNCE)?;
        out.push(record);
        i += 1;
    }
    Some(out)
}

fn parse_a11y_v4(data: &[u8]) -> Vec<A11yRecord> {
    let Some(count) = read_u32(data, 4) else {
        return Vec::new();
    };
    let Some(total) = count
        .checked_mul(A11Y_STRIDE_V4 as u32)
        .and_then(|n| n.checked_add(A11Y_HEADER_V4 as u32))
    else {
        return Vec::new();
    };
    if total as usize > data.len() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < count as usize {
        let o = A11Y_HEADER_V4 + i * A11Y_STRIDE_V4;
        let Some(id) = read_i32(data, o) else { break };
        let Some(parent) = read_i32(data, o + 4) else { break };
        let Some(role) = read_i32(data, o + 8) else { break };
        let Some(x) = read_i32(data, o + 12) else { break };
        let Some(y) = read_i32(data, o + 16) else { break };
        let Some(w) = read_i32(data, o + 20) else { break };
        let Some(h) = read_i32(data, o + 24) else { break };
        let Some(actions) = read_u32(data, o + 28) else { break };
        let Some(flags) = read_u32(data, o + 32) else { break };
        let Some(name_off) = read_u32(data, o + 36) else { break };
        let Some(name_len) = read_u32(data, o + 40) else { break };
        let Some(value_off) = read_u32(data, o + 44) else { break };
        let Some(value_len) = read_u32(data, o + 48) else { break };
        out.push(A11yRecord {
            id,
            parent,
            role,
            x,
            y,
            w,
            h,
            actions,
            flags,
            name: read_string(data, name_off, name_len),
            value: read_string(data, value_off, value_len),
            expanded: -1,
            invalid: 0,
            level: 0,
            live: 0,
            value_min: 0.0,
            value_max: 0.0,
            value_step: 0.0,
            description: String::new(),
            label_id: 0,
            described_by_id: 0,
            controls_id: 0,
            announce_seq: 0,
        });
        i += 1;
    }
    out
}

fn role_for(id: i32) -> Role {
    match id {
        4 | 5 => Role::Label,
        6 => Role::Button,
        7 => Role::Slider,
        8 => Role::Switch,
        9 | 10 | 13 => Role::RadioButton,
        11 => Role::TextInput,
        12 => Role::Dialog,
        14 => Role::List,
        15 => Role::ListItem,
        16 => Role::Canvas,
        21 => Role::Splitter,
        22 => Role::ProgressIndicator,
        23 => Role::Image,
        25 => Role::Tab,
        26 => Role::MenuBar,
        27 => Role::MenuItem,
        29 => Role::Link,
        _ => Role::GenericContainer,
    }
}

fn node_id(id: i32) -> NodeId {
    NodeId(id as u64 + 1)
}

fn root_record(records: &[A11yRecord]) -> Option<&A11yRecord> {
    records
        .iter()
        .find(|r| r.parent < 0)
        .or_else(|| records.first())
}

fn focus_record(records: &[A11yRecord]) -> Option<&A11yRecord> {
    records.iter().find(|r| r.flags & 4 != 0)
}

fn focus_id(records: &[A11yRecord]) -> Option<NodeId> {
    focus_record(records)
        .or_else(|| root_record(records))
        .map(|r| node_id(r.id))
}

fn build_node(records: &[A11yRecord], r: &A11yRecord) -> Node {
    let mut b = Node::new(role_for(r.role));
    if !r.name.is_empty() {
        b.set_label(r.name.clone());
    }
    if !r.value.is_empty() {
        b.set_value(r.value.clone());
    }
    b.set_bounds(Rect {
        x0: r.x as f64,
        y0: r.y as f64,
        x1: (r.x + r.w) as f64,
        y1: (r.y + r.h) as f64,
    });
    if r.actions & 1 != 0 {
        b.add_action(Action::Click);
    }
    if r.actions & 2 != 0 {
        b.add_action(Action::Click);
    }
    if r.actions & 4 != 0 {
        b.add_action(Action::SetValue);
    }
    if r.actions != 0 {
        b.add_action(Action::Focus);
    }
    if r.flags & 1 != 0 {
        b.set_toggled(Toggled::True);
    }
    if r.flags & 2 != 0 {
        b.set_disabled();
    }
    if r.flags & 8 != 0 {
        b.set_hidden();
    }
    if r.flags & 16 != 0 {
        b.set_selected(true);
    }
    if r.flags & 32 != 0 {
        b.set_busy();
    }
    if r.flags & 64 != 0 {
        b.set_required();
    }
    match r.expanded {
        1 => b.set_expanded(true),
        0 => b.set_expanded(false),
        _ => {}
    }
    // UI invalid codes are 1 invalid, 2 spelling, 3 grammar.
    match r.invalid {
        1 => b.set_invalid(Invalid::True),
        2 => b.set_invalid(Invalid::Spelling),
        3 => b.set_invalid(Invalid::Grammar),
        _ => {}
    }
    if r.level > 0 {
        b.set_level(r.level as usize);
    }
    match r.live {
        1 => b.set_live(Live::Polite),
        2 => b.set_live(Live::Assertive),
        _ => {}
    }
    if r.value_max > r.value_min {
        b.set_min_numeric_value(r.value_min);
        b.set_max_numeric_value(r.value_max);
        if r.value_step > 0.0 {
            b.set_numeric_value_step(r.value_step);
        }
        if let Ok(v) = r.value.trim().parse::<f64>() {
            b.set_numeric_value(v);
        }
    }
    if !r.description.is_empty() {
        b.set_description(r.description.clone());
    }
    if r.label_id > 0 {
        b.set_labelled_by([node_id(r.label_id)]);
    }
    if r.described_by_id > 0 {
        b.set_described_by([node_id(r.described_by_id)]);
    }
    if r.controls_id > 0 {
        b.set_controls([node_id(r.controls_id)]);
    }
    for child in records.iter().filter(|c| c.parent == r.id) {
        b.push_child(node_id(child.id));
    }
    b
}

fn build_a11y(records: &[A11yRecord]) -> Option<TreeUpdate> {
    if records.is_empty() {
        return None;
    }
    let root_rec = root_record(records)?;
    let root = node_id(root_rec.id);
    let focus = focus_id(records)?;
    let nodes: Vec<(NodeId, Node)> = records
        .iter()
        .map(|r| (node_id(r.id), build_node(records, r)))
        .collect();
    Some(TreeUpdate {
        nodes,
        tree: Some(Tree::new(root)),
        focus,
    })
}

fn same_content(a: &A11yRecord, b: &A11yRecord) -> bool {
    a.id == b.id
        && a.parent == b.parent
        && a.role == b.role
        && a.x == b.x
        && a.y == b.y
        && a.w == b.w
        && a.h == b.h
        && a.actions == b.actions
        && (a.flags & !4) == (b.flags & !4)
        && a.name == b.name
        && a.value == b.value
        && a.expanded == b.expanded
        && a.invalid == b.invalid
        && a.level == b.level
        && a.live == b.live
        && a.value_min.to_bits() == b.value_min.to_bits()
        && a.value_max.to_bits() == b.value_max.to_bits()
        && a.value_step.to_bits() == b.value_step.to_bits()
        && a.description == b.description
        && a.label_id == b.label_id
        && a.described_by_id == b.described_by_id
        && a.controls_id == b.controls_id
        && a.announce_seq == b.announce_seq
}

fn valid_structure(records: &[A11yRecord], map: &BTreeMap<i32, &A11yRecord>, root: i32) -> bool {
    for record in records {
        let mut id = record.id;
        let mut guard = 0;
        while id != root {
            let Some(current) = map.get(&id) else {
                return false;
            };
            if current.parent < 0 {
                return false;
            }
            id = current.parent;
            guard += 1;
            if guard > records.len() {
                return false;
            }
        }
    }
    true
}

pub fn diff_a11y(prev: &[A11yRecord], next: &[A11yRecord]) -> Option<TreeUpdate> {
    diff_a11y_with_focus(prev, focus_id(prev), next, focus_id(next))
}

fn diff_a11y_with_focus(
    prev: &[A11yRecord],
    prev_focus: Option<NodeId>,
    next: &[A11yRecord],
    next_focus: Option<NodeId>,
) -> Option<TreeUpdate> {
    if next.is_empty() {
        return None;
    }
    if prev.is_empty() {
        return build_a11y(next);
    }
    let prev_root = root_record(prev)?;
    let next_root = root_record(next)?;
    if prev_root.id != next_root.id {
        return build_a11y(next);
    }

    let prev_map: BTreeMap<i32, &A11yRecord> = prev.iter().map(|r| (r.id, r)).collect();
    let next_map: BTreeMap<i32, &A11yRecord> = next.iter().map(|r| (r.id, r)).collect();
    if prev_map.len() != prev.len()
        || next_map.len() != next.len()
        || !valid_structure(prev, &prev_map, prev_root.id)
        || !valid_structure(next, &next_map, next_root.id)
    {
        return build_a11y(next);
    }

    let mut changed = prev_map.len() != next_map.len() || prev_focus != next_focus;
    let mut include: BTreeSet<i32> = BTreeSet::new();
    include.insert(next_root.id);
    if let Some(record) = focus_record(next) {
        include.insert(record.id);
    }

    for (id, record) in &next_map {
        match prev_map.get(id) {
            Some(old) => {
                if !same_content(old, record) {
                    changed = true;
                    include.insert(*id);
                }
                if old.parent != record.parent {
                    changed = true;
                    include.insert(*id);
                    if old.parent >= 0 && next_map.contains_key(&old.parent) {
                        include.insert(old.parent);
                    }
                    if record.parent >= 0 {
                        include.insert(record.parent);
                    }
                }
            }
            None => {
                changed = true;
                include.insert(*id);
                if record.parent >= 0 {
                    include.insert(record.parent);
                }
            }
        }
    }

    for (id, old) in &prev_map {
        if !next_map.contains_key(id) {
            changed = true;
            if old.parent >= 0 && next_map.contains_key(&old.parent) {
                include.insert(old.parent);
            }
        }
    }

    if !changed {
        return None;
    }

    let mut nodes: Vec<(NodeId, Node)> = include
        .into_iter()
        .filter_map(|id| {
            next_map
                .get(&id)
                .map(|r| (node_id(id), build_node(next, r)))
        })
        .collect();
    nodes.sort_by_key(|(id, _)| id.0);
    Some(TreeUpdate {
        nodes,
        tree: None,
        focus: next_focus.unwrap_or_else(|| node_id(next_root.id)),
    })
}

struct A11yBridge {
    adapter: Adapter,
    latest: Arc<Mutex<Option<TreeUpdate>>>,
    actions: mpsc::Receiver<ActionRequest>,
    previous: Vec<A11yRecord>,
    previous_focus: Option<NodeId>,
}

impl A11yBridge {
    fn new() -> Self {
        let latest = Arc::new(Mutex::new(None));
        let (tx, actions) = mpsc::channel();
        let adapter = Adapter::new(
            A11yActivation(Arc::clone(&latest)),
            A11yActions(tx),
            A11yDeactivation,
        );
        Self {
            adapter,
            latest,
            actions,
            previous: Vec::new(),
            previous_focus: None,
        }
    }

    fn update(&mut self, data: &[u8]) -> usize {
        let records = parse_a11y(data);
        if records.is_empty() {
            self.previous.clear();
            self.previous_focus = None;
            return 0;
        }
        let next_focus = focus_id(&records);
        let update =
            diff_a11y_with_focus(&self.previous, self.previous_focus, &records, next_focus);
        let Some(update) = update else {
            return records.len();
        };
        self.previous = records.clone();
        self.previous_focus = next_focus;
        if let Some(full) = build_a11y(&records) {
            if let Ok(mut slot) = self.latest.lock() {
                *slot = Some(full);
            }
        }
        self.adapter.update_if_active(|| update);
        if std::env::var_os("RNX_TRACE").is_some() {
            eprintln!("a11y nodes={}", records.len());
        }
        records.len()
    }

    fn drain(&mut self) -> Vec<(i32, i32, Vec<u8>)> {
        let mut out = Vec::new();
        while let Ok(request) = self.actions.try_recv() {
            let node = request.target.0 as i32 - 1;
            match request.action {
                Action::Click => out.push((node, 0, Vec::new())),
                Action::Focus => out.push((node, 1, Vec::new())),
                Action::Increment => out.push((node, 2, Vec::new())),
                Action::Decrement => out.push((node, 3, Vec::new())),
                Action::SetValue => {
                    let text = match request.data {
                        Some(ActionData::Value(v)) => v.to_string(),
                        _ => String::new(),
                    };
                    out.push((node, 4, text.into_bytes()));
                }
                _ => {}
            }
        }
        out
    }
}

struct State {
    // The GPU surface must drop before the Wayland connection closes, or the
    // Vulkan swapchain teardown calls into a dead wl_display.
    gpu: Option<Gpu>,
    backend: i32,
    _compositor: wl_compositor::WlCompositor,
    _wm_base: xdg_wm_base::XdgWmBase,
    surface: wl_surface::WlSurface,
    _xdg_surface: xdg_surface::XdgSurface,
    _toplevel: xdg_toplevel::XdgToplevel,
    _seat: Option<wl_seat::WlSeat>,
    pointer: Option<wl_pointer::WlPointer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    touch: Option<wl_touch::WlTouch>,
    _shm: Option<wl_shm::WlShm>,
    pointer_enter_serial: u32,
    cursor_shape: i32,
    cursor_theme: Option<CursorTheme>,
    cursor_surface: Option<wl_surface::WlSurface>,
    conn: Connection,
    width: u32,
    height: u32,
    requested_width: u32,
    requested_height: u32,
    pending_width: u32,
    pending_height: u32,
    configured: bool,
    closed: bool,
    events: VecDeque<Event>,
    pointer_x: f64,
    pointer_y: f64,
    touches: BTreeMap<i32, (f64, f64)>,
    scroll_acc: f32,
    last_a: i32,
    last_b: i32,
    last_c: i32,
    generation: u32,
    xkb_keymap: Option<xkbcommon::xkb::Keymap>,
    xkb_state: Option<xkbcommon::xkb::State>,
    repeat_rate: i32,
    repeat_delay: i32,
    repeat_key: Option<u32>,
    repeat_text: Vec<u8>,
    repeat_next: Option<Instant>,
    text_buf: Vec<u8>,
    qh: QueueHandle<State>,
    frame_done: Arc<AtomicBool>,
    frame_cb: Option<wl_callback::WlCallback>,
    data_manager: Option<wl_data_device_manager::WlDataDeviceManager>,
    data_device: Option<wl_data_device::WlDataDevice>,
    clipboard_offer: Option<wl_data_offer::WlDataOffer>,
    clipboard_has_text: bool,
    clipboard_source: Option<wl_data_source::WlDataSource>,
    clipboard_text: String,
    owns_clipboard: bool,
    clipboard_gen: u32,
    last_serial: u32,
    text_input_manager: Option<ZwpTextInputManagerV3>,
    text_input: Option<ZwpTextInputV3>,
    ime_enabled: bool,
    ime_rect: (i32, i32, i32, i32),
    ime_preedit: String,
    ime_commit: String,
    ime_delete: (i32, i32),
    gamepads: Vec<File>,
    suspended: bool,
    pointer_constraints: Option<ZwpPointerConstraintsV1>,
    locked_pointer: Option<ZwpLockedPointerV1>,
    confined_pointer: Option<ZwpConfinedPointerV1>,
    a11y: Option<A11yBridge>,
}

struct Host {
    queue: EventQueue<State>,
    state: State,
    extras: Vec<Box<Host>>,
    event_window: u32,
}

impl Host {
    fn window(&mut self, id: u32) -> Option<&mut Host> {
        if id == 0 {
            return Some(self);
        }
        self.extras.get_mut(id as usize - 1).map(|boxed| boxed.as_mut())
    }

    fn window_width(&self, id: u32) -> i32 {
        if id == 0 {
            return self.state.width as i32;
        }
        match self.extras.get(id as usize - 1) {
            Some(h) => h.state.width as i32,
            None => -1,
        }
    }

    fn window_height(&self, id: u32) -> i32 {
        if id == 0 {
            return self.state.height as i32;
        }
        match self.extras.get(id as usize - 1) {
            Some(h) => h.state.height as i32,
            None => -1,
        }
    }

    fn window_generation(&self, id: u32) -> i32 {
        if id == 0 {
            return self.state.generation as i32;
        }
        match self.extras.get(id as usize - 1) {
            Some(h) => h.state.generation as i32,
            None => -1,
        }
    }
}

impl State {
    fn due_repeat(&mut self) {
        let Some(next) = self.repeat_next else {
            return;
        };
        if Instant::now() < next {
            return;
        }
        let Some(key) = self.repeat_key else {
            self.repeat_next = None;
            return;
        };
        self.events.push_back(Event::KeyDown(key as i32));
        if !self.repeat_text.is_empty() {
            self.events.push_back(Event::Text(self.repeat_text.clone()));
        }
        let rate = if self.repeat_rate > 0 { self.repeat_rate } else { 25 };
        let period = (1000 / rate).max(1) as u64;
        self.repeat_next = Some(Instant::now() + Duration::from_millis(period));
    }

    fn repeat_wait_ms(&self) -> i32 {
        let Some(next) = self.repeat_next else {
            return -1;
        };
        let now = Instant::now();
        if next <= now {
            return 0;
        }
        next.duration_since(now).as_millis().min(1000) as i32
    }

    fn apply_cursor(&mut self) {
        let Some(name) = cursor_name(self.cursor_shape) else {
            return;
        };
        if self.pointer_enter_serial == 0 {
            return;
        }
        let Some(pointer) = self.pointer.clone() else {
            return;
        };
        let Some(surface) = self.cursor_surface.clone() else {
            return;
        };
        let Some(theme) = self.cursor_theme.as_mut() else {
            return;
        };
        let Some(cursor) = theme.get_cursor(name) else {
            return;
        };
        if cursor.image_count() == 0 {
            return;
        }
        let buffer = &cursor[0];
        let (hotspot_x, hotspot_y) = buffer.hotspot();
        let (width, height) = buffer.dimensions();
        surface.attach(Some(buffer), 0, 0);
        surface.damage(0, 0, width as i32, height as i32);
        surface.commit();
        pointer.set_cursor(
            self.pointer_enter_serial,
            Some(&surface),
            hotspot_x as i32,
            hotspot_y as i32,
        );
    }

    fn release_pointer_capture(&mut self) {
        if let Some(pointer) = self.locked_pointer.take() {
            pointer.destroy();
        }
        if let Some(pointer) = self.confined_pointer.take() {
            pointer.destroy();
        }
        let _ = self.conn.flush();
    }

    fn pointer_capture(&mut self, mode: i32) -> bool {
        self.release_pointer_capture();
        if mode == 0 {
            return true;
        }
        let Some(constraints) = self.pointer_constraints.as_ref() else {
            return false;
        };
        let Some(pointer) = self.pointer.as_ref() else {
            return false;
        };
        if mode == 1 {
            let locked = constraints.lock_pointer(
                &self.surface,
                pointer,
                None,
                zwp_pointer_constraints_v1::Lifetime::Persistent,
                &self.qh,
                (),
            );
            self.locked_pointer = Some(locked);
        } else {
            let confined = constraints.confine_pointer(
                &self.surface,
                pointer,
                None,
                zwp_pointer_constraints_v1::Lifetime::Persistent,
                &self.qh,
                (),
            );
            self.confined_pointer = Some(confined);
        }
        let _ = self.conn.flush();
        true
    }

    fn drain_a11y(&mut self) {
        if let Some(bridge) = self.a11y.as_mut() {
            for (node, action, bytes) in bridge.drain() {
                self.events.push_back(Event::A11yAction(node, action, bytes));
            }
        }
    }

    fn read_gamepads(&mut self) {
        let mut idx = 0;
        while idx < self.gamepads.len() {
            let mut buf = [0u8; 64];
            let n = {
                let file = &mut self.gamepads[idx];
                file.read(&mut buf).unwrap_or(0)
            };
            if n > 0 && std::env::var_os("RNX_TRACE").is_some() {
                eprintln!("gamepad read {n}");
            }
            let mut at = 0;
            while at + 8 <= n {
                let value = i16::from_ne_bytes([buf[at + 4], buf[at + 5]]);
                let kind = buf[at + 6];
                let number = buf[at + 7] as i32;
                if kind & 0x01 != 0 {
                    let down = if value != 0 { 1 } else { 0 };
                    if std::env::var_os("RNX_TRACE").is_some() {
                        eprintln!("gamepad button {number}={down}");
                    }
                    self.events.push_back(Event::GamepadButton(number, down));
                } else if kind & 0x02 != 0 {
                    if std::env::var_os("RNX_TRACE").is_some() {
                        eprintln!("gamepad axis {number}={value}");
                    }
                    self.events
                        .push_back(Event::GamepadAxis(number, value as i32));
                }
                at += 8;
            }
            idx += 1;
        }
    }

    fn ime_enable(&mut self, x: i32, y: i32, w: i32, h: i32) -> bool {
        let Some(input) = self.text_input.as_ref() else {
            return false;
        };
        if !self.ime_enabled {
            input.enable();
            input.set_content_type(
                zwp_text_input_v3::ContentHint::None,
                zwp_text_input_v3::ContentPurpose::Normal,
            );
            input.set_cursor_rectangle(x, y, w, h);
            self.ime_rect = (x, y, w, h);
            self.ime_enabled = true;
        } else if self.ime_rect != (x, y, w, h) {
            input.set_cursor_rectangle(x, y, w, h);
            self.ime_rect = (x, y, w, h);
        }
        input.commit();
        let _ = self.conn.flush();
        true
    }

    fn ime_disable(&mut self) {
        if let Some(input) = self.text_input.as_ref() {
            if self.ime_enabled {
                input.disable();
                input.commit();
                let _ = self.conn.flush();
            }
        }
        self.ime_enabled = false;
        self.ime_preedit.clear();
        self.ime_commit.clear();
    }

    fn clipboard_put(&mut self, text: String) -> bool {
        let Some(manager) = self.data_manager.as_ref() else {
            return false;
        };
        let Some(device) = self.data_device.as_ref() else {
            return false;
        };
        let source = manager.create_data_source(&self.qh, ());
        source.offer("text/plain;charset=utf-8".to_string());
        source.offer("text/plain".to_string());
        self.clipboard_text = text;
        self.clipboard_source = Some(source.clone());
        self.owns_clipboard = true;
        device.set_selection(Some(&source), self.last_serial);
        let _ = self.conn.flush();
        true
    }

    fn clipboard_get(&mut self) -> String {
        if self.owns_clipboard {
            return self.clipboard_text.clone();
        }
        if !self.clipboard_has_text {
            return String::new();
        }
        let Some(offer) = self.clipboard_offer.as_ref() else {
            return String::new();
        };
        let mut fds = [0i32; 2];
        if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
            return String::new();
        }
        let read_file = unsafe { File::from_raw_fd(fds[0]) };
        let write_fd = unsafe { OwnedFd::from_raw_fd(fds[1]) };
        offer.receive("text/plain;charset=utf-8".to_string(), write_fd.as_fd());
        let _ = self.conn.flush();
        let mut pfd = libc::pollfd {
            fd: fds[0],
            events: libc::POLLIN,
            revents: 0,
        };
        if unsafe { libc::poll(&mut pfd, 1, 1000) } <= 0 {
            return String::new();
        }
        let mut out = Vec::new();
        let mut file = read_file;
        let _ = file.take(1_048_576).read_to_end(&mut out);
        String::from_utf8_lossy(&out).into_owned()
    }

    fn create_gpu(&mut self) -> bool {
        if self.gpu.is_some() {
            return true;
        }
        let Some(display) = NonNull::new(self.conn.backend().display_ptr() as *mut c_void) else {
            return false;
        };
        let Some(window) = NonNull::new(self.surface.id().as_ptr() as *mut c_void) else {
            return false;
        };
        let raw_display = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(display));
        let raw_window = RawWindowHandle::Wayland(WaylandWindowHandle::new(window));
        let Some((resources, Some(surface))) =
            open_resources(self.backend, Some((raw_display, raw_window)))
        else {
            return false;
        };
        let caps = surface.get_capabilities(&resources.adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| *f == wgpu::TextureFormat::Rgba8Unorm)
            .or_else(|| {
                caps.formats
                    .iter()
                    .copied()
                    .find(|f| *f == wgpu::TextureFormat::Bgra8Unorm)
            })
            .or_else(|| caps.formats.first().copied())
            .unwrap_or(wgpu::TextureFormat::Bgra8Unorm);
        let width = self.width.max(1);
        let height = self.height.max(1);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_DST,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
                wgpu::CompositeAlphaMode::Opaque
            } else {
                caps.alpha_modes
                    .first()
                    .copied()
                    .unwrap_or(wgpu::CompositeAlphaMode::Auto)
            },
            view_formats: vec![],
        };
        surface.configure(&resources.device, &config);
        let (target, target_view) = make_target_format(&resources.device, width, height, format);
        self.gpu = Some(Gpu {
            resources,
            surface,
            config,
            target,
            target_view,
            scratch: None,
        });
        self.generation += 1;
        true
    }

    fn resize_gpu(&mut self) {
        let width = self.width.max(1);
        let height = self.height.max(1);
        let Some(gpu) = &mut self.gpu else {
            return;
        };
        if gpu.config.width == width && gpu.config.height == height {
            return;
        }
        if std::env::var_os("RNX_TRACE").is_some() {
            eprintln!("target {}x{}", width, height);
        }
        gpu.config.width = width;
        gpu.config.height = height;
        gpu.surface.configure(&gpu.resources.device, &gpu.config);
        let (target, view) =
            make_target_format(&gpu.resources.device, width, height, gpu.config.format);
        gpu.target = target;
        gpu.target_view = view;
        self.generation += 1;
    }

    fn rebuild(&mut self) {
        self.gpu = None;
        if self.create_gpu() {
            self.events.push_back(Event::SurfaceLost);
        }
    }

    fn present(&mut self, bytes: &[u8], rects: &[(i32, i32, i32, i32)]) -> i32 {
        if !self.configured {
            return -2;
        }
        if self.gpu.is_none() && !self.create_gpu() {
            return -3;
        }
        let started = std::time::Instant::now();
        let commands = match scene::parse(bytes) {
            Ok(commands) => commands,
            Err(code) => return code,
        };
        let parsed = started.elapsed();
        if rects.is_empty() {
            return 0;
        }
        let Some(gpu) = self.gpu.as_mut() else {
            return -3;
        };
        let surface_w = gpu.config.width;
        let surface_h = gpu.config.height;
        let full = rects.len() == 1
            && rects[0].0 <= 0
            && rects[0].1 <= 0
            && rects[0].0 + rects[0].2 >= surface_w as i32
            && rects[0].1 + rects[0].3 >= surface_h as i32;
        let acquired = started.elapsed();
        let frame = match gpu.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                drop(frame);
                if std::env::var_os("RNX_TRACE").is_some() {
                    eprintln!("surface suboptimal: reconfigure");
                }
                gpu.surface.configure(&gpu.resources.device, &gpu.config);
                match gpu.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(frame)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
                    wgpu::CurrentSurfaceTexture::Outdated => {
                        gpu.surface.configure(&gpu.resources.device, &gpu.config);
                        return -4;
                    }
                    wgpu::CurrentSurfaceTexture::Lost => {
                        self.rebuild();
                        return -4;
                    }
                    wgpu::CurrentSurfaceTexture::Timeout
                    | wgpu::CurrentSurfaceTexture::Occluded => return 1,
                    wgpu::CurrentSurfaceTexture::Validation => return -4,
                }
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                gpu.surface.configure(&gpu.resources.device, &gpu.config);
                return -4;
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.rebuild();
                return -4;
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return 1;
            }
            wgpu::CurrentSurfaceTexture::Validation => return -4,
        };

        if full {
            if render_commands(
                &gpu.resources,
                &commands,
                &gpu.target_view,
                surface_w,
                surface_h,
                Affine::IDENTITY,
            )
            .is_err()
            {
                return -5;
            }
        } else {
            let max_w = rects.iter().map(|r| r.2.max(1) as u32).max().unwrap_or(1);
            let max_h = rects.iter().map(|r| r.3.max(1) as u32).max().unwrap_or(1);
            let grow = gpu
                .scratch
                .as_ref()
                .map_or(true, |(_, _, w, h)| *w < max_w || *h < max_h);
            if grow {
                let (texture, view) =
                    make_target_format(&gpu.resources.device, max_w, max_h, gpu.config.format);
                gpu.scratch = Some((texture, view, max_w, max_h));
            }
            let (scratch, scratch_view, sw, sh) = gpu.scratch.as_ref().unwrap();
            for rect in rects {
                if render_commands(
                    &gpu.resources,
                    &commands,
                    scratch_view,
                    *sw,
                    *sh,
                    Affine::translate((-(rect.0 as f64), -(rect.1 as f64))),
                )
                .is_err()
                {
                    return -5;
                }
                let mut encoder = gpu
                    .resources
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
                encoder.copy_texture_to_texture(
                    scratch.as_image_copy(),
                    wgpu::TexelCopyTextureInfo {
                        texture: &gpu.target,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: rect.0.max(0) as u32,
                            y: rect.1.max(0) as u32,
                            z: 0,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::Extent3d {
                        width: rect.2.max(1) as u32,
                        height: rect.3.max(1) as u32,
                        depth_or_array_layers: 1,
                    },
                );
                gpu.resources.queue.submit(Some(encoder.finish()));
            }
        }

        let mut encoder = gpu
            .resources
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_texture_to_texture(
            gpu.target.as_image_copy(),
            frame.texture.as_image_copy(),
            wgpu::Extent3d {
                width: surface_w,
                height: surface_h,
                depth_or_array_layers: 1,
            },
        );
        gpu.resources.queue.submit(Some(encoder.finish()));
        gpu.resources.queue.present(frame);
        let _ = gpu.resources.device.poll(wgpu::PollType::Poll);
        if std::env::var_os("RNX_TRACE").is_some() {
            for rect in rects {
                eprintln!(
                    "  region {} {} {} {}",
                    rect.0, rect.1, rect.2, rect.3
                );
            }
            eprintln!(
                "present rects={} parse={:?} acquire={:?} total={:?}",
                rects.len(),
                parsed,
                acquired - parsed,
                started.elapsed()
            );
        }
        0
    }
}

impl Host {
    fn pump_blocking(&mut self) {
        self.state.read_gamepads();
        self.state.drain_a11y();
        let _ = self.queue.dispatch_pending(&mut self.state);
        self.state.due_repeat();
        if !self.state.events.is_empty() {
            return;
        }
        let _ = self.queue.flush();
        let Some(guard) = self.queue.prepare_read() else {
            return;
        };
        let timeout = self.state.repeat_wait_ms();
        let mut fds: Vec<libc::pollfd> = vec![libc::pollfd {
            fd: self.state.conn.as_fd().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        }];
        for file in &self.state.gamepads {
            fds.push(libc::pollfd {
                fd: file.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            });
        }
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as u64, timeout) };
        if ready > 0 {
            if fds[0].revents & libc::POLLIN != 0 {
                let _ = guard.read();
            }
            self.state.read_gamepads();
        }
        let _ = self.queue.dispatch_pending(&mut self.state);
        self.state.due_repeat();
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(State: ignore wl_compositor::WlCompositor);
delegate_noop!(State: ignore wl_surface::WlSurface);
delegate_noop!(State: ignore wl_shm::WlShm);

impl Dispatch<wl_callback::WlCallback, Arc<AtomicBool>> for State {
    fn event(
        _: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        done: &Arc<AtomicBool>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            done.store(true, Ordering::SeqCst);
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for State {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for State {
    fn event(
        state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
            let old_width = state.width;
            let old_height = state.height;
            state.width = state.pending_width.max(1);
            state.height = state.pending_height.max(1);
            state.configured = true;
            if state.gpu.is_some() {
                state.resize_gpu();
            } else {
                state.create_gpu();
            }
            if state.width != old_width || state.height != old_height {
                state
                    .events
                    .push_back(Event::Resize(state.width as i32, state.height as i32));
            }
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for State {
    fn event(
        state: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            xdg_toplevel::Event::Configure { width, height, states } => {
                let mut suspended = false;
                for raw in &states {
                    if *raw == xdg_toplevel::State::Suspended as u8 {
                        suspended = true;
                    }
                }
                if suspended != state.suspended {
                    state.suspended = suspended;
                    if suspended {
                        state.events.push_back(Event::Suspended);
                    } else {
                        state.events.push_back(Event::Resumed);
                    }
                }
                state.pending_width = if width > 0 {
                    width as u32
                } else {
                    state.requested_width
                };
                state.pending_height = if height > 0 {
                    height as u32
                } else {
                    state.requested_height
                };
            }
            xdg_toplevel::Event::Close => {
                state.closed = true;
                state.events.push_back(Event::Close);
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(caps),
        } = event
        {
            if caps.contains(wl_seat::Capability::Pointer) && state.pointer.is_none() {
                state.pointer = Some(seat.get_pointer(qh, ()));
            }
            if caps.contains(wl_seat::Capability::Keyboard) && state.keyboard.is_none() {
                state.keyboard = Some(seat.get_keyboard(qh, ()));
            }
            if caps.contains(wl_seat::Capability::Touch) && state.touch.is_none() {
                state.touch = Some(seat.get_touch(qh, ()));
            }
        }
    }
}

fn key_text_bytes(text: &str) -> Vec<u8> {
    if text.is_empty() || text.chars().any(|c| c.is_control()) {
        Vec::new()
    } else {
        text.as_bytes().to_vec()
    }
}

fn scroll_pixels(acc: &mut f32, value: f32) -> i32 {
    *acc += value * 4.0;
    let whole = acc.trunc();
    *acc -= whole;
    whole as i32
}

fn release_touch(
    touches: &mut BTreeMap<i32, (f64, f64)>,
    events: &mut VecDeque<Event>,
    id: i32,
) {
    let (x, y) = touches.remove(&id).unwrap_or((0.0, 0.0));
    events.push_back(Event::TouchUp(id, x as i32, y as i32));
}

fn cancel_touches(touches: &mut BTreeMap<i32, (f64, f64)>, events: &mut VecDeque<Event>) {
    let ids: Vec<i32> = touches.keys().copied().collect();
    touches.clear();
    for id in ids {
        events.push_back(Event::TouchCancel(id));
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter {
                serial,
                surface_x,
                surface_y,
                ..
            } => {
                state.pointer_enter_serial = serial;
                state.pointer_x = surface_x;
                state.pointer_y = surface_y;
                state
                    .events
                    .push_back(Event::PointerMove(surface_x as i32, surface_y as i32));
                state.apply_cursor();
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                state.pointer_x = surface_x;
                state.pointer_y = surface_y;
                state
                    .events
                    .push_back(Event::PointerMove(surface_x as i32, surface_y as i32));
            }
            wl_pointer::Event::Button {
                serial,
                button,
                state: WEnum::Value(wl_pointer::ButtonState::Pressed),
                ..
            } if button == BTN_LEFT => {
                state.last_serial = serial;
                state.events.push_back(Event::PointerDown(
                    state.pointer_x as i32,
                    state.pointer_y as i32,
                ));
            }
            wl_pointer::Event::Button {
                serial,
                button,
                state: WEnum::Value(wl_pointer::ButtonState::Released),
                ..
            } if button == BTN_LEFT => {
                state.last_serial = serial;
                state.events.push_back(Event::PointerUp(0, 0));
            }
            wl_pointer::Event::Leave { .. } => {
                state.events.push_back(Event::PointerLeave);
            }
            wl_pointer::Event::Axis {
                axis: WEnum::Value(wl_pointer::Axis::VerticalScroll),
                value,
                ..
            } => {
                let px = scroll_pixels(&mut state.scroll_acc, value as f32);
                if px != 0 {
                    state.events.push_back(Event::Scroll(px));
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Keymap {
                format: WEnum::Value(wl_keyboard::KeymapFormat::XkbV1),
                fd,
                size,
            } => {
                let mut file: File = fd.into();
                let _ = file.seek(SeekFrom::Start(0));
                let mut buf = Vec::with_capacity(size as usize);
                if file.read_to_end(&mut buf).is_ok() {
                    let text = String::from_utf8_lossy(&buf);
                    let text = text.trim_end_matches('\0').to_string();
                    let ctx = xkbcommon::xkb::Context::new(0);
                    if let Some(keymap) = xkbcommon::xkb::Keymap::new_from_string(
                        &ctx,
                        text,
                        xkbcommon::xkb::KEYMAP_FORMAT_TEXT_V1,
                        0,
                    ) {
                        state.xkb_state = Some(xkbcommon::xkb::State::new(&keymap));
                        state.xkb_keymap = Some(keymap);
                        state.events.push_back(Event::TextMode);
                    }
                }
            }
            wl_keyboard::Event::RepeatInfo { rate, delay } => {
                state.repeat_rate = rate;
                state.repeat_delay = delay;
                if rate <= 0 {
                    state.repeat_next = None;
                }
            }
            wl_keyboard::Event::Key {
                serial,
                key,
                state: WEnum::Value(key_state),
                ..
            } => {
                state.last_serial = serial;
                let code = key as i32;
                let down = matches!(key_state, wl_keyboard::KeyState::Pressed);
                let mut text = String::new();
                if let Some(xkb) = state.xkb_state.as_mut() {
                    let kc = xkbcommon::xkb::Keycode::new(key + 8);
                    let dir = if down {
                        xkbcommon::xkb::KeyDirection::Down
                    } else {
                        xkbcommon::xkb::KeyDirection::Up
                    };
                    xkb.update_key(kc, dir);
                    if down {
                        text = xkb.key_get_utf8(kc);
                    }
                }
                if !text.is_empty() && std::env::var_os("RNX_TRACE").is_some() {
                    eprintln!("text={text:?}");
                }
                if down {
                    state.events.push_back(Event::KeyDown(code));
                    let bytes = key_text_bytes(&text);
                    if !bytes.is_empty() {
                        state.events.push_back(Event::Text(bytes.clone()));
                    }
                    let modifier = matches!(
                        key,
                        1 | 15 | 29 | 42 | 54 | 56 | 58 | 97 | 100 | 125 | 126
                    );
                    if !modifier && state.repeat_rate > 0 {
                        state.repeat_key = Some(key);
                        state.repeat_text = bytes;
                        let delay = state.repeat_delay.max(1) as u64;
                        state.repeat_next =
                            Some(Instant::now() + Duration::from_millis(delay));
                    }
                } else {
                    state.events.push_back(Event::KeyUp(code));
                    if state.repeat_key == Some(key) {
                        state.repeat_key = None;
                        state.repeat_next = None;
                    }
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_touch::WlTouch, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_touch::WlTouch,
        event: wl_touch::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_touch::Event::Down { id, x, y, .. } => {
                state.touches.insert(id, (x, y));
                state
                    .events
                    .push_back(Event::TouchDown(id, x as i32, y as i32));
            }
            wl_touch::Event::Motion { id, x, y, .. } => {
                state.touches.insert(id, (x, y));
                state
                    .events
                    .push_back(Event::TouchMove(id, x as i32, y as i32));
            }
            wl_touch::Event::Up { id, .. } => {
                release_touch(&mut state.touches, &mut state.events, id);
            }
            wl_touch::Event::Cancel => {
                cancel_touches(&mut state.touches, &mut state.events);
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwpTextInputV3, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwpTextInputV3,
        event: zwp_text_input_v3::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_text_input_v3::Event::PreeditString { text, .. } => {
                let text = text.unwrap_or_default();
                state.ime_preedit = text.clone();
                state
                    .events
                    .push_back(Event::ImePreedit(text.into_bytes()));
            }
            zwp_text_input_v3::Event::CommitString { text } => {
                let text = text.unwrap_or_default();
                state.ime_preedit.clear();
                state
                    .events
                    .push_back(Event::ImeCommit(text.into_bytes()));
            }
            zwp_text_input_v3::Event::DeleteSurroundingText {
                before_length,
                after_length,
            } => {
                state.ime_delete = (before_length as i32, after_length as i32);
                state
                    .events
                    .push_back(Event::ImeDelete(before_length as i32, after_length as i32));
            }
            zwp_text_input_v3::Event::Leave { .. } => {
                state.ime_enabled = false;
                state.ime_preedit.clear();
                state.ime_commit.clear();
                state.events.push_back(Event::ImeReset);
            }
            _ => {}
        }
    }
}

delegate_noop!(State: ignore ZwpTextInputManagerV3);
delegate_noop!(State: ignore ZwpPointerConstraintsV1);
delegate_noop!(State: ignore ZwpLockedPointerV1);
delegate_noop!(State: ignore ZwpConfinedPointerV1);

impl Dispatch<wl_data_device::WlDataDevice, ()> for State {
    wayland_client::event_created_child!(State, wl_data_device::WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (wl_data_offer::WlDataOffer, ())
    ]);

    fn event(
        state: &mut Self,
        _: &wl_data_device::WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::DataOffer { id } => {
                if std::env::var_os("RNX_TRACE").is_some() {
                    eprintln!("clipboard data offer");
                }
                state.clipboard_offer = Some(id);
                state.clipboard_has_text = false;
            }
            wl_data_device::Event::Selection { id } => {
                if std::env::var_os("RNX_TRACE").is_some() {
                    eprintln!("clipboard selection");
                }
                if let Some(offer) = id {
                    state.clipboard_offer = Some(offer);
                } else {
                    state.clipboard_offer = None;
                    state.clipboard_has_text = false;
                }
                state.clipboard_gen = state.clipboard_gen.wrapping_add(1);
                state.events.push_back(Event::Clipboard);
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_data_offer::WlDataOffer, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_data_offer::WlDataOffer,
        event: wl_data_offer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_data_offer::Event::Offer { mime_type } = event {
            if std::env::var_os("RNX_TRACE").is_some() {
                eprintln!("clipboard mime {mime_type}");
            }
            if mime_type.starts_with("text/plain") {
                state.clipboard_has_text = true;
            }
        }
    }
}

impl Dispatch<wl_data_source::WlDataSource, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_data_source::WlDataSource,
        event: wl_data_source::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_source::Event::Send { mime_type, fd } => {
                if mime_type.starts_with("text/plain") {
                    let mut file: File = fd.into();
                    let _ = file.write_all(state.clipboard_text.as_bytes());
                }
            }
            wl_data_source::Event::Cancelled => {
                state.clipboard_source = None;
                state.owns_clipboard = false;
            }
            _ => {}
        }
    }
}

delegate_noop!(State: ignore wl_data_device_manager::WlDataDeviceManager);

fn cursor_name(shape: i32) -> Option<&'static str> {
    match shape {
        0 => Some("default"),
        1 => Some("text"),
        2 => Some("pointer"),
        3 => Some("crosshair"),
        4 => Some("nwse-resize"),
        _ => None,
    }
}

fn cursor_size() -> u32 {
    std::env::var("XCURSOR_SIZE")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .map(|size| size.clamp(16, 64))
        .unwrap_or(24)
}

fn cursor_theme_name() -> String {
    std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "default".to_string())
}

fn open(title: &str, width: u32, height: u32, backend: i32) -> Option<Host> {
    let conn = Connection::connect_to_env().ok()?;
    let (globals, queue) = registry_queue_init::<State>(&conn).ok()?;
    let qh = queue.handle();
    let compositor = globals
        .bind::<wl_compositor::WlCompositor, State, ()>(&qh, 4..=6, ())
        .ok()?;
    let wm_base = globals
        .bind::<xdg_wm_base::XdgWmBase, State, ()>(&qh, 1..=6, ())
        .ok()?;
    let seat = globals
        .bind::<wl_seat::WlSeat, State, ()>(&qh, 1..=9, ())
        .ok();
    let surface = compositor.create_surface(&qh, ());
    let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg_surface.get_toplevel(&qh, ());
    toplevel.set_title(title.to_string());
    toplevel.set_app_id("dev.rasmalai.app".to_string());
    surface.commit();

    let text_input_manager = globals
        .bind::<ZwpTextInputManagerV3, State, _>(&qh, 1..=1, ())
        .ok();
    let text_input = match (&text_input_manager, &seat) {
        (Some(manager), Some(seat)) => Some(manager.get_text_input(seat, &qh, ())),
        _ => None,
    };
    let pointer_constraints = globals
        .bind::<ZwpPointerConstraintsV1, State, _>(&qh, 1..=1, ())
        .ok();
    let data_manager = globals
        .bind::<wl_data_device_manager::WlDataDeviceManager, State, _>(&qh, 1..=3, ())
        .ok();
    let data_device = match (&data_manager, &seat) {
        (Some(manager), Some(seat)) => Some(manager.get_data_device(seat, &qh, ())),
        _ => None,
    };
    let shm = globals
        .bind::<wl_shm::WlShm, State, _>(&qh, 1..=1, ())
        .ok();
    let cursor_surface = Some(compositor.create_surface(&qh, ()));
    let cursor_theme = match shm.as_ref() {
        Some(shm) => CursorTheme::load_from_name(
            &conn,
            shm.clone(),
            &cursor_theme_name(),
            cursor_size(),
        )
        .ok(),
        None => None,
    };
    let mut gamepads = Vec::new();
    let mut gi = 0;
    while gi < 8 {
        let path = format!("/dev/input/js{gi}");
        if let Ok(file) = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
        {
            gamepads.push(file);
        }
        gi += 1;
    }
    let gamepad_count = gamepads.len();
    if std::env::var_os("RNX_TRACE").is_some() {
        eprintln!(
            "clipboard manager={} device={} text_input={} gamepads={} constraints={}",
            data_manager.is_some(),
            data_device.is_some(),
            text_input.is_some(),
            gamepad_count,
            pointer_constraints.is_some()
        );
    }
    let mut host = Host {
        queue,
        state: State {
            conn,
            surface,
            _xdg_surface: xdg_surface,
            _toplevel: toplevel,
            _seat: seat,
            pointer: None,
            keyboard: None,
            touch: None,
            _shm: shm,
            pointer_enter_serial: 0,
            cursor_shape: 0,
            cursor_theme,
            cursor_surface,
            gpu: None,
            backend,
            _compositor: compositor,
            _wm_base: wm_base,
            width,
            height,
            requested_width: width,
            requested_height: height,
            pending_width: width,
            pending_height: height,
            configured: false,
            closed: false,
            events: VecDeque::new(),
            pointer_x: 0.0,
            pointer_y: 0.0,
            touches: BTreeMap::new(),
            scroll_acc: 0.0,
            last_a: 0,
            last_b: 0,
            last_c: 0,
            generation: 0,
            xkb_keymap: None,
            xkb_state: None,
            repeat_rate: 25,
            repeat_delay: 600,
            repeat_key: None,
            repeat_text: Vec::new(),
            repeat_next: None,
            text_buf: Vec::new(),
            qh: qh.clone(),
            frame_done: Arc::new(AtomicBool::new(false)),
            frame_cb: None,
            data_manager,
            data_device,
            clipboard_offer: None,
            clipboard_has_text: false,
            clipboard_source: None,
            clipboard_text: String::new(),
            owns_clipboard: false,
            clipboard_gen: 0,
            last_serial: 0,
            text_input_manager,
            text_input,
            ime_enabled: false,
            ime_rect: (0, 0, 0, 0),
            ime_preedit: String::new(),
            ime_commit: String::new(),
            ime_delete: (0, 0),
            gamepads,
            pointer_constraints,
            locked_pointer: None,
            confined_pointer: None,
            suspended: false,
            a11y: Some(A11yBridge::new()),
        },
        extras: Vec::new(),
        event_window: 0,
    };
    if gamepad_count > 0 {
        host.state.events.push_back(Event::GamepadConnected);
    }
    while !host.state.configured && !host.state.closed {
        host.pump_blocking();
    }
    if host.state.closed {
        return None;
    }
    Some(host)
}

#[no_mangle]
pub extern "C" fn host_open(
    title: *const u8,
    title_len: i32,
    width: i32,
    height: i32,
    backend: i32,
) -> *mut c_void {
    if width <= 0 || height <= 0 {
        return std::ptr::null_mut();
    }
    let title = if title.is_null() || title_len <= 0 {
        "Rasmalai app".to_string()
    } else {
        let bytes = unsafe { slice::from_raw_parts(title, title_len as usize) };
        String::from_utf8_lossy(bytes).into_owned()
    };
    match open(&title, width as u32, height as u32, backend) {
        Some(host) => Box::into_raw(Box::new(host)) as *mut c_void,
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
fn pump_child(child: &mut Host, timeout: i32) {
    let fd = child.state.conn.as_fd().as_raw_fd();
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    if unsafe { libc::poll(&mut pfd, 1, timeout) } > 0 {
        if let Some(guard) = child.queue.prepare_read() {
            let _ = guard.read();
        }
    }
    let Host { queue, state, .. } = child;
    let _ = queue.dispatch_pending(state);
}

fn pick_event_window(primary: bool, extras: &[bool]) -> Option<usize> {
    if primary {
        return Some(0);
    }
    let mut i = 0;
    while i < extras.len() {
        if extras[i] {
            return Some(i + 1);
        }
        i += 1;
    }
    None
}

fn next_event_any(host: &mut Host) -> Option<i32> {
    let flags: Vec<bool> = host.extras.iter().map(|h| !h.state.events.is_empty()).collect();
    let Some(at) = pick_event_window(!host.state.events.is_empty(), &flags) else {
        return None;
    };
    host.event_window = at as u32;
    if at == 0 {
        return next_event(host);
    }
    next_event(host.extras[at - 1].as_mut())
}

fn encode_event(event: Event, text_buf: &mut Vec<u8>, clipboard_gen: u32) -> (i32, i32, i32, i32) {
    match event {
        Event::Close => (1, 0, 0, 0),
        Event::Resize(w, h) => (2, w, h, 0),
        Event::PointerDown(x, y) => (3, x, y, 0),
        Event::PointerUp(x, y) => (4, x, y, 0),
        Event::PointerMove(x, y) => (5, x, y, 0),
        Event::KeyDown(k) => (6, k, 0, 0),
        Event::KeyUp(k) => (7, k, 0, 0),
        Event::SurfaceLost => (8, 0, 0, 0),
        Event::Scroll(d) => {
            if std::env::var_os("RNX_TRACE").is_some() {
                eprintln!("scroll step={d}");
            }
            (9, d, 0, 0)
        }
        Event::PointerLeave => (10, 0, 0, 0),
        Event::Text(bytes) => {
            *text_buf = bytes;
            (11, text_buf.len() as i32, 0, 0)
        }
        Event::TextMode => (12, 1, 0, 0),
        Event::Clipboard => (13, clipboard_gen as i32, 0, 0),
        Event::ImePreedit(bytes) => {
            *text_buf = bytes;
            (14, text_buf.len() as i32, 0, 0)
        }
        Event::ImeCommit(bytes) => {
            *text_buf = bytes;
            (15, text_buf.len() as i32, 0, 0)
        }
        Event::ImeDelete(before, after) => (16, before, after, 0),
        Event::ImeReset => (17, 0, 0, 0),
        Event::GamepadAxis(index, value) => (18, index, value, 0),
        Event::GamepadButton(index, down) => (19, index, down, 0),
        Event::GamepadConnected => (20, 1, 0, 0),
        Event::TouchDown(id, x, y) => (25, id, x, y),
        Event::TouchMove(id, x, y) => (26, id, x, y),
        Event::TouchUp(id, x, y) => (27, id, x, y),
        Event::TouchCancel(id) => (28, id, 0, 0),
        Event::Suspended => (23, 1, 0, 0),
        Event::Resumed => (24, 1, 0, 0),
        Event::A11yAction(node, action, bytes) => {
            if action == 4 {
                *text_buf = bytes;
                (22, node, text_buf.len() as i32, 0)
            } else {
                (21, node, action, 0)
            }
        }
    }
}

struct EventQueueView<'a> {
    events: &'a mut VecDeque<Event>,
    text_buf: &'a mut Vec<u8>,
    clipboard_gen: u32,
    last_a: &'a mut i32,
    last_b: &'a mut i32,
    last_c: &'a mut i32,
}

impl EventQueueView<'_> {
    fn next_event(&mut self) -> Option<i32> {
        let event = self.events.pop_front()?;
        let (code, a, b, c) = encode_event(event, self.text_buf, self.clipboard_gen);
        *self.last_a = a;
        *self.last_b = b;
        *self.last_c = c;
        Some(code)
    }
}

fn next_event(host: &mut Host) -> Option<i32> {
    let state = &mut host.state;
    let clipboard_gen = state.clipboard_gen;
    EventQueueView {
        events: &mut state.events,
        text_buf: &mut state.text_buf,
        clipboard_gen,
        last_a: &mut state.last_a,
        last_b: &mut state.last_b,
        last_c: &mut state.last_c,
    }
    .next_event()
}

#[no_mangle]
pub extern "C" fn host_pump(handle: *mut c_void) -> i64 {
    let Some(host) = (unsafe { (handle as *mut Host).as_mut() }) else {
        return -1;
    };
    loop {
        if let Some(code) = next_event(host) {
            return code as i64;
        }
        if host.state.closed {
            return 1;
        }
        host.pump_blocking();
    }
}

/// Return the next input event. `block != 0` waits for one, which is what a
/// paused app does. `block == 0` drains what has arrived and returns 0, so a
/// running animation keeps drawing without waiting on input.
#[no_mangle]
pub extern "C" fn host_pump_timeout(handle: *mut c_void, block: i32) -> i64 {
    let Some(host) = (unsafe { (handle as *mut Host).as_mut() }) else {
        return -1;
    };
    if block != 0 {
        return host_pump(handle);
    }
    if let Some(code) = next_event_any(host) {
        return code as i64;
    }
    if host.state.closed {
        return 1;
    }
    let Host { queue, state, .. } = host;
    if let Some(guard) = queue.prepare_read() {
        let _ = guard.read();
    }
    let _ = queue.dispatch_pending(state);
    let mut i = 0;
    while i < host.extras.len() {
        pump_child(host.extras[i].as_mut(), 0);
        i += 1;
    }
    next_event_any(host).unwrap_or(0) as i64
}

/// Wait until the previously presented buffer is free for reuse.
///
/// The Wayland swapchain already paces acquisition, so this is a no-op here.
/// A surface socket host blocks on frame_done instead.
/// Pump events for up to `ms` milliseconds. Returns an input event when one
/// arrives, otherwise 0. This is the animation frame budget: it keeps a
/// running app near the compositor rate instead of spinning on an empty queue.
#[no_mangle]
pub extern "C" fn host_pump_for(handle: *mut c_void, ms: i32) -> i64 {
    let Some(host) = (unsafe { (handle as *mut Host).as_mut() }) else {
        return -1;
    };
    let _ = host.queue.dispatch_pending(&mut host.state);
    let _ = host.queue.flush();
    let fd = host.state.conn.as_fd().as_raw_fd();
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let ready = unsafe { libc::poll(&mut pfd, 1, ms.max(0)) };
    if ready > 0 && pfd.revents & libc::POLLIN != 0 {
        if let Some(guard) = host.queue.prepare_read() {
            let _ = guard.read();
        }
    }
    for extra in 0..host.extras.len() {
        pump_child(host.extras[extra].as_mut(), 0);
    }
    let _ = host.queue.dispatch_pending(&mut host.state);
    host.state.due_repeat();
    next_event_any(host).unwrap_or(0) as i64
}

#[no_mangle]
pub extern "C" fn host_request_frame(handle: *mut c_void) -> i64 {
    let Some(host) = (unsafe { (handle as *mut Host).as_mut() }) else {
        return -1;
    };
    if host.state.frame_cb.is_some() {
        return 0;
    }
    host.state.frame_done.store(false, Ordering::SeqCst);
    let cb = host
        .state
        .surface
        .frame(&host.state.qh, host.state.frame_done.clone());
    host.state.frame_cb = Some(cb);
    let _ = host.queue.flush();
    0
}

#[no_mangle]
pub extern "C" fn host_wait_frame(handle: *mut c_void, ms: i32) -> i64 {
    let Some(host) = (unsafe { (handle as *mut Host).as_mut() }) else {
        return -1;
    };
    let deadline = Instant::now() + Duration::from_millis(ms.max(0) as u64);
    loop {
        let _ = host.queue.dispatch_pending(&mut host.state);
        if host.state.frame_done.swap(false, Ordering::SeqCst) {
            let _ = host.state.frame_cb.take();
            break;
        }
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        let remain = (deadline - now).as_millis().min(20) as i32;
        let fd = host.state.conn.as_fd().as_raw_fd();
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut pfd, 1, remain.max(0)) };
        if ready > 0 && pfd.revents & libc::POLLIN != 0 {
            if let Some(guard) = host.queue.prepare_read() {
                let _ = guard.read();
            }
        }
    }
    next_event_any(host).unwrap_or(0) as i64
}

#[no_mangle]
pub extern "C" fn host_wait(handle: *mut c_void) -> i64 {
    if unsafe { (handle as *mut Host).as_ref() }.is_none() {
        return -1;
    }
    0
}

#[no_mangle]
pub extern "C" fn host_event_a(handle: *mut c_void) -> i64 {
    unsafe { (handle as *mut Host).as_ref() }
        .map(|h| h.state.last_a as i64)
        .unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn host_event_b(handle: *mut c_void) -> i64 {
    unsafe { (handle as *mut Host).as_ref() }
        .map(|h| h.state.last_b as i64)
        .unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn host_event_c(handle: *mut c_void) -> i64 {
    unsafe { (handle as *mut Host).as_ref() }
        .map(|h| h.state.last_c as i64)
        .unwrap_or(0)
}

/// Changes whenever the host recreates its target texture. The app forces a
/// full frame when this differs from the last value it saw.
#[no_mangle]
pub extern "C" fn host_generation(handle: *mut c_void) -> i64 {
    unsafe { (handle as *mut Host).as_ref() }
        .map(|h| h.state.generation as i64)
        .unwrap_or(-1)
}

#[no_mangle]
pub extern "C" fn host_width(handle: *mut c_void) -> i64 {
    unsafe { (handle as *mut Host).as_ref() }
        .map(|h| h.state.width as i64)
        .unwrap_or(-1)
}

#[no_mangle]
pub extern "C" fn host_height(handle: *mut c_void) -> i64 {
    unsafe { (handle as *mut Host).as_ref() }
        .map(|h| h.state.height as i64)
        .unwrap_or(-1)
}

#[no_mangle]
pub extern "C" fn host_present(
    handle: *mut c_void,
    list: *const u8,
    len: i32,
    damage: *const i32,
    count: i32,
) -> i64 {
    let Some(host) = (unsafe { (handle as *mut Host).as_mut() }) else {
        return -1;
    };
    if list.is_null() || len < 0 || count < 0 {
        return -1;
    }
    if count > 0 && damage.is_null() {
        return -1;
    }
    let bytes = unsafe { slice::from_raw_parts(list, len as usize) };
    let values = if count > 0 {
        unsafe { slice::from_raw_parts(damage, count as usize * 4) }
    } else {
        &[]
    };
    let mut rects = Vec::with_capacity(values.len() / 4);
    for r in values.chunks_exact(4) {
        let x = r[0].max(0).min(host.state.width as i32);
        let y = r[1].max(0).min(host.state.height as i32);
        let x1 = (r[0] + r[2]).max(0).min(host.state.width as i32);
        let y1 = (r[1] + r[3]).max(0).min(host.state.height as i32);
        if x1 > x && y1 > y {
            rects.push((x, y, x1 - x, y1 - y));
        }
    }
    host.state.present(bytes, &rects) as i64
}

#[no_mangle]
pub extern "C" fn host_open_window(
    handle: *mut c_void,
    title: *const u8,
    len: i32,
    width: u32,
    height: u32,
) -> i64 {
    let Some(host) = (unsafe { (handle as *mut Host).as_mut() }) else {
        return -1;
    };
    if title.is_null() || len < 0 {
        return -1;
    }
    if host.extras.len() >= 16 {
        return -2;
    }
    let bytes = unsafe { slice::from_raw_parts(title, len as usize) };
    let name = String::from_utf8_lossy(bytes).into_owned();
    let backend = host.state.backend;
    match open(&name, width, height, backend) {
        Some(child) => {
            host.extras.push(Box::new(child));
            host.extras.len() as i64
        }
        None => -3,
    }
}

#[no_mangle]
pub extern "C" fn host_window_count(handle: *mut c_void) -> i64 {
    let Some(host) = (unsafe { (handle as *mut Host).as_ref() }) else {
        return -1;
    };
    1 + host.extras.len() as i64
}

#[no_mangle]
pub extern "C" fn host_window_width(handle: *mut c_void, id: u32) -> i64 {
    let Some(host) = (unsafe { (handle as *mut Host).as_ref() }) else {
        return -1;
    };
    host.window_width(id) as i64
}

#[no_mangle]
pub extern "C" fn host_window_height(handle: *mut c_void, id: u32) -> i64 {
    let Some(host) = (unsafe { (handle as *mut Host).as_ref() }) else {
        return -1;
    };
    host.window_height(id) as i64
}

#[no_mangle]
pub extern "C" fn host_window_generation(handle: *mut c_void, id: u32) -> i64 {
    let Some(host) = (unsafe { (handle as *mut Host).as_ref() }) else {
        return -1;
    };
    host.window_generation(id) as i64
}

#[no_mangle]
pub extern "C" fn host_event_window(handle: *mut c_void) -> i64 {
    let Some(host) = (unsafe { (handle as *mut Host).as_ref() }) else {
        return 0;
    };
    host.event_window as i64
}

#[no_mangle]
pub extern "C" fn host_present_window(
    handle: *mut c_void,
    id: u32,
    list: *const u8,
    len: i32,
    damage: *const i32,
    count: i32,
) -> i64 {
    let Some(host) = (unsafe { (handle as *mut Host).as_mut() }) else {
        return -1;
    };
    if list.is_null() || len < 0 || count < 0 {
        return -1;
    }
    let Some(window) = host.window(id) else {
        return -5;
    };
    if count > 0 && damage.is_null() {
        return -1;
    }
    let bytes = unsafe { slice::from_raw_parts(list, len as usize) };
    let values = if count > 0 {
        unsafe { slice::from_raw_parts(damage, count as usize * 4) }
    } else {
        &[]
    };
    let mut rects = Vec::with_capacity(values.len() / 4);
    for r in values.chunks_exact(4) {
        let x = r[0].max(0).min(window.state.width as i32);
        let y = r[1].max(0).min(window.state.height as i32);
        let x1 = (r[0] + r[2]).max(0).min(window.state.width as i32);
        let y1 = (r[1] + r[3]).max(0).min(window.state.height as i32);
        if x1 > x && y1 > y {
            rects.push((x, y, x1 - x, y1 - y));
        }
    }
    window.state.present(bytes, &rects) as i64
}

#[no_mangle]
pub extern "C" fn host_close(handle: *mut c_void) {
    if handle.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(handle as *mut Host));
    }
}

#[no_mangle]
pub extern "C" fn host_surface_socket_open(
    path: *const u8,
    len: i32,
    id: u32,
    width: u32,
    height: u32,
) -> *mut c_void {
    if path.is_null() || len < 0 {
        return std::ptr::null_mut();
    }
    let bytes = unsafe { slice::from_raw_parts(path, len as usize) };
    let Ok(path) = std::str::from_utf8(bytes) else {
        return std::ptr::null_mut();
    };
    match surface_socket::SurfaceSocketHost::connect(path, id, width, height) {
        Ok(host) => Box::into_raw(Box::new(host)) as *mut c_void,
        Err(_) => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn host_surface_socket_commit(
    handle: *mut c_void,
    list: *const u8,
    len: i32,
    packet: *const u8,
    packet_len: i32,
    damage: *const i32,
    count: i32,
) -> i64 {
    let Some(host) = (unsafe { (handle as *mut surface_socket::SurfaceSocketHost).as_mut() }) else {
        return -1;
    };
    if list.is_null() || len < 0 || packet_len < 0 || count < 0 {
        return -1;
    }
    if count > 0 && damage.is_null() {
        return -1;
    }
    let list = unsafe { slice::from_raw_parts(list, len as usize) };
    let packet = if packet_len > 0 {
        unsafe { slice::from_raw_parts(packet, packet_len as usize) }
    } else {
        &[]
    };
    let values = if count > 0 {
        unsafe { slice::from_raw_parts(damage, count as usize * 4) }
    } else {
        &[]
    };
    let mut rects = Vec::with_capacity(values.len() / 4);
    for r in values.chunks_exact(4) {
        rects.push((r[0], r[1], r[2], r[3]));
    }
    match host.commit(list, packet, &rects) {
        Ok(status) => status as i64,
        Err(_) => -4,
    }
}

#[no_mangle]
pub extern "C" fn host_surface_socket_test_start(path: *const u8, len: i32) -> i64 {
    if path.is_null() || len < 0 {
        return -1;
    }
    let bytes = unsafe { slice::from_raw_parts(path, len as usize) };
    let Ok(path) = std::str::from_utf8(bytes) else {
        return -1;
    };
    match surface_socket::start_test_server(path) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

#[no_mangle]
pub extern "C" fn host_surface_socket_test_frames(path: *const u8, len: i32) -> i64 {
    surface_socket::test_lookup(path, len, |stats| stats.frames as i64)
}

#[no_mangle]
pub extern "C" fn host_surface_socket_test_last_id(path: *const u8, len: i32) -> i64 {
    surface_socket::test_lookup(path, len, |stats| stats.last_id as i64)
}

#[no_mangle]
pub extern "C" fn host_surface_socket_test_last_list(path: *const u8, len: i32) -> i64 {
    surface_socket::test_lookup(path, len, |stats| stats.last_list as i64)
}

#[no_mangle]
pub extern "C" fn host_surface_socket_test_last_packet(path: *const u8, len: i32) -> i64 {
    surface_socket::test_lookup(path, len, |stats| stats.last_packet as i64)
}

#[no_mangle]
pub extern "C" fn host_surface_socket_id(handle: *mut c_void) -> i64 {
    let Some(host) = (unsafe { (handle as *mut surface_socket::SurfaceSocketHost).as_ref() }) else {
        return -1;
    };
    host.id() as i64
}

#[no_mangle]
pub extern "C" fn host_surface_socket_close(handle: *mut c_void) {
    if handle.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(handle as *mut surface_socket::SurfaceSocketHost));
    }
}

#[no_mangle]
pub extern "C" fn host_abi_version() -> i64 {
    1
}

#[no_mangle]
pub extern "C" fn host_register_font(
    id: i32,
    name: *const u8,
    name_len: i32,
    data: *const u8,
    data_len: i32,
) -> i64 {
    if name.is_null() || data.is_null() || name_len < 0 || data_len < 0 {
        return -1;
    }
    let name = unsafe { slice::from_raw_parts(name, name_len as usize) };
    let data = unsafe { slice::from_raw_parts(data, data_len as usize) };
    let Ok(name) = std::str::from_utf8(name) else {
        return -1;
    };
    scene::register_font(id as u32, name, data) as i64
}

#[no_mangle]
pub extern "C" fn host_address(handle: *mut c_void) -> i64 {
    handle as i64
}

struct FrameHeader {
    count: u32,
    width: u32,
    height: u32,
    scale: u32,
    color: u32,
}

fn parse_frame_packet(data: &[u8]) -> Option<FrameHeader> {
    if data.len() < 48 {
        return None;
    }
    if read_u32(data, 0)? != 424242 {
        return None;
    }
    if read_u32(data, 4)? != 1 {
        return None;
    }
    Some(FrameHeader {
        count: read_u32(data, 8)?,
        width: read_u32(data, 16)?,
        height: read_u32(data, 20)?,
        scale: read_u32(data, 24)?,
        color: read_u32(data, 28)?,
    })
}

#[no_mangle]
pub extern "C" fn host_frame_packet(handle: *mut c_void, data: *const u8, len: i32) -> i64 {
    if data.is_null() || len < 0 {
        return -1;
    }
    let bytes = unsafe { slice::from_raw_parts(data, len as usize) };
    let Some(header) = parse_frame_packet(bytes) else {
        return -1;
    };
    if std::env::var_os("RNX_TRACE").is_some() {
        eprintln!(
            "frame packet nodes={} size={}x{} scale={} color={}",
            header.count, header.width, header.height, header.scale, header.color
        );
    }
    let _ = handle;
    header.count as i64
}

#[no_mangle]
pub extern "C" fn host_a11y(handle: *mut c_void, data: *const u8, len: i32) -> i64 {
    let Some(host) = (unsafe { (handle as *mut Host).as_mut() }) else {
        return -1;
    };
    if data.is_null() || len < 0 {
        return -1;
    }
    let bytes = unsafe { slice::from_raw_parts(data, len as usize) };
    let Some(bridge) = host.state.a11y.as_mut() else {
        return -1;
    };
    bridge.update(bytes) as i64
}

#[no_mangle]
pub extern "C" fn host_pointer_capture(handle: *mut c_void, mode: i32) -> i64 {
    let Some(host) = (unsafe { (handle as *mut Host).as_mut() }) else {
        return -1;
    };
    if host.state.pointer_capture(mode) { 0 } else { -1 }
}

#[no_mangle]
pub extern "C" fn host_set_cursor(handle: *mut c_void, shape: i32) -> i64 {
    let Some(host) = (unsafe { (handle as *mut Host).as_mut() }) else {
        return -1;
    };
    if !(0..=4).contains(&shape) {
        return -1;
    }
    host.state.cursor_shape = shape;
    host.state.apply_cursor();
    let _ = host.state.conn.flush();
    0
}

#[no_mangle]
pub extern "C" fn host_ime_enable(
    handle: *mut c_void,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) -> i64 {
    let Some(host) = (unsafe { (handle as *mut Host).as_mut() }) else {
        return -1;
    };
    if host.state.ime_enable(x, y, w, h) { 0 } else { -1 }
}

#[no_mangle]
pub extern "C" fn host_ime_disable(handle: *mut c_void) {
    if let Some(host) = unsafe { (handle as *mut Host).as_mut() } {
        host.state.ime_disable();
    }
}

#[no_mangle]
pub extern "C" fn host_clipboard_set(handle: *mut c_void, text: *const u8, len: i32) -> i64 {
    let Some(host) = (unsafe { (handle as *mut Host).as_mut() }) else {
        return -1;
    };
    if text.is_null() || len < 0 {
        return -1;
    }
    let bytes = unsafe { slice::from_raw_parts(text, len as usize) };
    let s = String::from_utf8_lossy(bytes).into_owned();
    if host.state.clipboard_put(s) { 0 } else { -1 }
}

#[no_mangle]
pub extern "C" fn host_clipboard_get(handle: *mut c_void, out: *mut u8, cap: i32) -> i64 {
    let Some(host) = (unsafe { (handle as *mut Host).as_mut() }) else {
        return -1;
    };
    if out.is_null() || cap <= 0 {
        return 0;
    }
    let s = host.state.clipboard_get();
    let bytes = s.as_bytes();
    let n = bytes.len().min(cap as usize);
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), out, n);
    }
    n as i64
}

#[no_mangle]
pub extern "C" fn host_clipboard_generation(handle: *mut c_void) -> i64 {
    let Some(host) = (unsafe { (handle as *mut Host).as_ref() }) else {
        return 0;
    };
    host.state.clipboard_gen as i64
}

#[no_mangle]
pub extern "C" fn host_text_byte(handle: *mut c_void, index: i32) -> i64 {
    let Some(host) = (unsafe { (handle as *mut Host).as_ref() }) else {
        return 0;
    };
    host.state
        .text_buf
        .get(index as usize)
        .copied()
        .unwrap_or(0) as i64
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

#[no_mangle]
pub extern "C" fn host_render_offscreen(
    w: i32,
    h: i32,
    list: *const u8,
    len: i32,
    out: *mut u8,
) -> i64 {
    if w <= 0 || h <= 0 || list.is_null() || out.is_null() || len < 0 {
        return -1;
    }
    let bytes = unsafe { slice::from_raw_parts(list, len as usize) };
    let commands = match scene::parse(bytes) {
        Ok(commands) => commands,
        Err(code) => return code as i64,
    };
    let Some(core) = core(0) else {
        return -3;
    };
    let (texture, view) = make_target(&core.device, w as u32, h as u32);
    if render_commands(
        core,
        &commands,
        &view,
        w as u32,
        h as u32,
        Affine::IDENTITY,
    )
    .is_err()
    {
        return -5;
    }
    readback(&core.device, &core.queue, &texture, w as u32, h as u32, out) as i64
}

/// Copy the last presented surface into `out` as RGBA8, row-major.
/// Returns the byte count, or a negative required size when `cap` is short.
#[no_mangle]
pub extern "C" fn host_capture(handle: *mut c_void, out: *mut u8, cap: i32) -> i64 {
    let Some(host) = (unsafe { (handle as *mut Host).as_mut() }) else {
        return -1;
    };
    if out.is_null() || cap < 0 {
        return -1;
    }
    let Some(gpu) = host.state.gpu.as_ref() else {
        return -2;
    };
    let w = gpu.config.width;
    let h = gpu.config.height;
    let need = (w as i64) * (h as i64) * 4;
    if (cap as i64) < need {
        return -need;
    }
    let rc = readback(&gpu.resources.device, &gpu.resources.queue, &gpu.target, w, h, out);
    if rc != 0 {
        return rc as i64;
    }
    need
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xkb_keymap_types_and_shifts() {
        let ctx = xkbcommon::xkb::Context::new(0);
        let keymap = xkbcommon::xkb::Keymap::new_from_names(&ctx, "", "", "us", "", None, 0)
            .expect("us keymap");
        let mut state = xkbcommon::xkb::State::new(&keymap);
        let a = xkbcommon::xkb::Keycode::new(30 + 8);
        let shift = xkbcommon::xkb::Keycode::new(42 + 8);
        state.update_key(a, xkbcommon::xkb::KeyDirection::Down);
        assert_eq!(state.key_get_utf8(a), "a");
        state.update_key(a, xkbcommon::xkb::KeyDirection::Up);
        state.update_key(shift, xkbcommon::xkb::KeyDirection::Down);
        state.update_key(a, xkbcommon::xkb::KeyDirection::Down);
        assert_eq!(state.key_get_utf8(a), "A");
    }

    #[test]
    fn cursor_shapes_map_to_names() {
        assert_eq!(cursor_name(0), Some("default"));
        assert_eq!(cursor_name(1), Some("text"));
        assert_eq!(cursor_name(2), Some("pointer"));
        assert_eq!(cursor_name(3), Some("crosshair"));
        assert_eq!(cursor_name(4), Some("nwse-resize"));
        assert_eq!(cursor_name(-1), None);
        assert_eq!(cursor_name(5), None);
    }

    fn put_u32(out: &mut [u8], at: usize, value: u32) {
        out[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put_f64(out: &mut [u8], at: usize, value: f64) {
        out[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn blur_list() -> Vec<u8> {
        const HEADER: usize = 32;
        const STRIDE: usize = 64;
        let blob_at = HEADER + 3 * STRIDE;
        let mut out = vec![0u8; blob_at];
        put_u32(&mut out, 0, 0x4458_4E52);
        put_u32(&mut out, 4, 3);
        put_u32(&mut out, 8, 3);
        put_u32(&mut out, 12, STRIDE as u32);
        put_u32(&mut out, 16, blob_at as u32);
        put_u32(&mut out, 20, 0);

        let at = HEADER;
        put_u32(&mut out, at, 21);
        put_f64(&mut out, at + 16, 0.0);
        put_f64(&mut out, at + 24, 0.0);
        put_f64(&mut out, at + 32, 64.0);
        put_f64(&mut out, at + 40, 64.0);
        put_f64(&mut out, at + 48, 3.0);

        let at = HEADER + STRIDE;
        put_u32(&mut out, at, 1);
        put_u32(&mut out, at + 4, 0xFF00_00FF);
        put_f64(&mut out, at + 16, 24.0);
        put_f64(&mut out, at + 24, 24.0);
        put_f64(&mut out, at + 32, 16.0);
        put_f64(&mut out, at + 40, 16.0);

        put_u32(&mut out, HEADER + 2 * STRIDE, 22);
        out
    }

    #[test]
    fn offscreen_blur_spreads_group_pixels() {
        let list = blur_list();
        let mut out = vec![0u8; 64 * 64 * 4];
        let code = host_render_offscreen(
            64,
            64,
            list.as_ptr(),
            list.len() as i32,
            out.as_mut_ptr(),
        );
        if code == -3 {
            eprintln!("skip: no GPU device available for the blur test");
            return;
        }
        assert_eq!(code, 0, "offscreen render failed");
        let pixel = |x: usize, y: usize| {
            let at = (y * 64 + x) * 4;
            [out[at], out[at + 1], out[at + 2], out[at + 3]]
        };
        let center = pixel(32, 32);
        assert!(center[0] > 200 && center[3] > 200, "center {center:?}");
        let edge = pixel(41, 32);
        assert!(edge[0] > 0 && edge[3] > 0, "blur spread {edge:?}");
        let corner = pixel(0, 0);
        assert_eq!(corner[3], 0, "far corner {corner:?}");
    }
}

#[cfg(test)]
mod a11y_tests {
    use super::*;

    fn v4_record(
        id: i32,
        parent: i32,
        role: i32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        actions: u32,
        flags: u32,
        name_off: u32,
        name_len: u32,
        value_off: u32,
        value_len: u32,
    ) -> Vec<u8> {
        let mut rec = Vec::new();
        rec.extend_from_slice(&(id as u32).to_le_bytes());
        rec.extend_from_slice(&(parent as u32).to_le_bytes());
        rec.extend_from_slice(&(role as u32).to_le_bytes());
        rec.extend_from_slice(&(x as u32).to_le_bytes());
        rec.extend_from_slice(&(y as u32).to_le_bytes());
        rec.extend_from_slice(&(w as u32).to_le_bytes());
        rec.extend_from_slice(&(h as u32).to_le_bytes());
        rec.extend_from_slice(&actions.to_le_bytes());
        rec.extend_from_slice(&flags.to_le_bytes());
        rec.extend_from_slice(&name_off.to_le_bytes());
        rec.extend_from_slice(&name_len.to_le_bytes());
        rec.extend_from_slice(&value_off.to_le_bytes());
        rec.extend_from_slice(&value_len.to_le_bytes());
        rec
    }

    fn v4_packet() -> Vec<u8> {
        let strings_start = A11Y_HEADER_V4 + 2 * A11Y_STRIDE_V4;
        let mut out = Vec::new();
        out.extend_from_slice(&A11Y_MAGIC.to_le_bytes());
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&v4_record(0, -1, 1, 0, 0, 200, 100, 0, 0, 0, 0, 0, 0));
        out.extend_from_slice(&v4_record(
            1,
            0,
            6,
            10,
            10,
            80,
            32,
            1,
            0,
            strings_start as u32,
            3,
            (strings_start + 3) as u32,
            0,
        ));
        out.extend_from_slice(b"Add");
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn v5_record(
        id: i32,
        parent: i32,
        role: i32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        actions: u32,
        flags: u32,
        name_off: u32,
        name_len: u32,
        value_off: u32,
        value_len: u32,
        expanded: i32,
        invalid: u32,
        level: u32,
        live: u32,
        value_min: f64,
        value_max: f64,
        value_step: f64,
        desc_len: u32,
    ) -> Vec<u8> {
        let mut rec = Vec::new();
        rec.extend_from_slice(&(id as u32).to_le_bytes());
        rec.extend_from_slice(&(parent as u32).to_le_bytes());
        rec.extend_from_slice(&(role as u32).to_le_bytes());
        rec.extend_from_slice(&(x as u32).to_le_bytes());
        rec.extend_from_slice(&(y as u32).to_le_bytes());
        rec.extend_from_slice(&(w as u32).to_le_bytes());
        rec.extend_from_slice(&(h as u32).to_le_bytes());
        rec.extend_from_slice(&actions.to_le_bytes());
        rec.extend_from_slice(&flags.to_le_bytes());
        rec.extend_from_slice(&name_off.to_le_bytes());
        rec.extend_from_slice(&name_len.to_le_bytes());
        rec.extend_from_slice(&value_off.to_le_bytes());
        rec.extend_from_slice(&value_len.to_le_bytes());
        rec.extend_from_slice(&(expanded as u32).to_le_bytes());
        rec.extend_from_slice(&invalid.to_le_bytes());
        rec.extend_from_slice(&level.to_le_bytes());
        rec.extend_from_slice(&live.to_le_bytes());
        rec.extend_from_slice(&value_min.to_le_bytes());
        rec.extend_from_slice(&value_max.to_le_bytes());
        rec.extend_from_slice(&value_step.to_le_bytes());
        rec.extend_from_slice(&desc_len.to_le_bytes());
        assert_eq!(rec.len(), A11Y_STRIDE_V5);
        rec
    }

    fn v5_packet() -> Vec<u8> {
        let strings_start = A11Y_HEADER_V5 + 2 * A11Y_STRIDE_V5;
        let mut out = Vec::new();
        out.extend_from_slice(&A11Y_MAGIC.to_le_bytes());
        out.extend_from_slice(&A11Y_VERSION_V5.to_le_bytes());
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&(A11Y_STRIDE_V5 as u32).to_le_bytes());
        out.extend_from_slice(&v5_record(
            0, -1, 1, 0, 0, 320, 240, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0.0, 0.0, 0.0, 0,
        ));
        out.extend_from_slice(&v5_record(
            1,
            0,
            6,
            10,
            10,
            80,
            32,
            1,
            16 | 32 | 64,
            strings_start as u32,
            4,
            (strings_start + 4) as u32,
            3,
            1,
            2,
            2,
            2,
            0.0,
            10.0,
            0.5,
            14,
        ));
        out.extend_from_slice(b"Save0.5primary action");
        out
    }

    struct V5Node<'a> {
        id: i32,
        parent: i32,
        role: i32,
        actions: u32,
        flags: u32,
        name: &'a str,
        value: &'a str,
        live: u32,
    }

    fn v5_packet_with(nodes: &[V5Node<'_>]) -> Vec<u8> {
        let strings_start = A11Y_HEADER_V5 + nodes.len() * A11Y_STRIDE_V5;
        let mut out = Vec::new();
        out.extend_from_slice(&A11Y_MAGIC.to_le_bytes());
        out.extend_from_slice(&A11Y_VERSION_V5.to_le_bytes());
        out.extend_from_slice(&(nodes.len() as u32).to_le_bytes());
        out.extend_from_slice(&(A11Y_STRIDE_V5 as u32).to_le_bytes());
        let mut strings = Vec::new();
        for node in nodes {
            let name_off = strings_start + strings.len();
            strings.extend_from_slice(node.name.as_bytes());
            let value_off = strings_start + strings.len();
            strings.extend_from_slice(node.value.as_bytes());
            out.extend_from_slice(&v5_record(
                node.id,
                node.parent,
                node.role,
                0,
                0,
                10,
                10,
                node.actions,
                node.flags,
                name_off as u32,
                node.name.len() as u32,
                value_off as u32,
                node.value.len() as u32,
                -1,
                0,
                0,
                node.live,
                0.0,
                0.0,
                0.0,
                0,
            ));
        }
        out.extend_from_slice(&strings);
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn v6_record(
        id: i32,
        parent: i32,
        role: i32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        actions: u32,
        flags: u32,
        name_off: u32,
        name_len: u32,
        value_off: u32,
        value_len: u32,
        expanded: i32,
        invalid: u32,
        level: u32,
        live: u32,
        value_min: f64,
        value_max: f64,
        value_step: f64,
        desc_len: u32,
        label_id: i32,
        described_by_id: i32,
        controls_id: i32,
        announce_seq: u32,
    ) -> Vec<u8> {
        let mut rec = v5_record(
            id, parent, role, x, y, w, h, actions, flags, name_off, name_len, value_off, value_len,
            expanded, invalid, level, live, value_min, value_max, value_step, desc_len,
        );
        rec.extend_from_slice(&(label_id as u32).to_le_bytes());
        rec.extend_from_slice(&(described_by_id as u32).to_le_bytes());
        rec.extend_from_slice(&(controls_id as u32).to_le_bytes());
        rec.extend_from_slice(&announce_seq.to_le_bytes());
        assert_eq!(rec.len(), A11Y_STRIDE_V6);
        rec
    }

    struct V6Node<'a> {
        id: i32,
        parent: i32,
        role: i32,
        actions: u32,
        flags: u32,
        name: &'a str,
        value: &'a str,
        live: u32,
        label_id: i32,
        described_by_id: i32,
        controls_id: i32,
        announce_seq: u32,
    }

    fn v6_node(id: i32, parent: i32, role: i32) -> V6Node<'static> {
        V6Node {
            id,
            parent,
            role,
            actions: 0,
            flags: 0,
            name: "",
            value: "",
            live: 0,
            label_id: 0,
            described_by_id: 0,
            controls_id: 0,
            announce_seq: 0,
        }
    }

    fn v6_packet_with(nodes: &[V6Node<'_>]) -> Vec<u8> {
        let strings_start = A11Y_HEADER_V6 + nodes.len() * A11Y_STRIDE_V6;
        let mut out = Vec::new();
        out.extend_from_slice(&A11Y_MAGIC.to_le_bytes());
        out.extend_from_slice(&A11Y_VERSION_V6.to_le_bytes());
        out.extend_from_slice(&(nodes.len() as u32).to_le_bytes());
        out.extend_from_slice(&(A11Y_STRIDE_V6 as u32).to_le_bytes());
        let mut strings = Vec::new();
        for node in nodes {
            let name_off = strings_start + strings.len();
            strings.extend_from_slice(node.name.as_bytes());
            let value_off = strings_start + strings.len();
            strings.extend_from_slice(node.value.as_bytes());
            out.extend_from_slice(&v6_record(
                node.id,
                node.parent,
                node.role,
                0,
                0,
                10,
                10,
                node.actions,
                node.flags,
                name_off as u32,
                node.name.len() as u32,
                value_off as u32,
                node.value.len() as u32,
                -1,
                0,
                0,
                node.live,
                0.0,
                0.0,
                0.0,
                0,
                node.label_id,
                node.described_by_id,
                node.controls_id,
                node.announce_seq,
            ));
        }
        out.extend_from_slice(&strings);
        out
    }

    fn node_by_id(update: &TreeUpdate, id: u64) -> &Node {
        update
            .nodes
            .iter()
            .find(|(node, _)| *node == NodeId(id))
            .map(|(_, node)| node)
            .expect("node")
    }

    fn root() -> V5Node<'static> {
        V5Node {
            id: 0,
            parent: -1,
            role: 1,
            actions: 0,
            flags: 0,
            name: "",
            value: "",
            live: 0,
        }
    }

    #[test]
    fn identical_packets_produce_no_update() {
        let packet = v5_packet_with(&[
            root(),
            V5Node {
                id: 1,
                parent: 0,
                role: 7,
                actions: 4,
                flags: 0,
                name: "Volume",
                value: "1",
                live: 0,
            },
        ]);
        let records = parse_a11y(&packet);
        assert_eq!(records.len(), 2);
        assert!(diff_a11y(&records, &records).is_none());
    }

    #[test]
    fn value_change_skips_unchanged_siblings() {
        let before = v5_packet_with(&[
            root(),
            V5Node {
                id: 1,
                parent: 0,
                role: 7,
                actions: 4,
                flags: 0,
                name: "Volume",
                value: "1",
                live: 0,
            },
            V5Node {
                id: 2,
                parent: 0,
                role: 6,
                actions: 1,
                flags: 0,
                name: "Other",
                value: "",
                live: 0,
            },
        ]);
        let after = v5_packet_with(&[
            root(),
            V5Node {
                id: 1,
                parent: 0,
                role: 7,
                actions: 4,
                flags: 0,
                name: "Volume",
                value: "2",
                live: 0,
            },
            V5Node {
                id: 2,
                parent: 0,
                role: 6,
                actions: 1,
                flags: 0,
                name: "Other",
                value: "",
                live: 0,
            },
        ]);
        let update = diff_a11y(&parse_a11y(&before), &parse_a11y(&after)).expect("update");
        assert_eq!(node_by_id(&update, 2).value(), Some("2"));
        assert!(!update.nodes.iter().any(|(id, _)| *id == NodeId(3)));
        assert_eq!(
            update
                .nodes
                .iter()
                .filter(|(id, _)| *id != NodeId(1))
                .count(),
            1
        );
    }

    #[test]
    fn focus_change_produces_focus_only_update() {
        let before = v5_packet_with(&[
            root(),
            V5Node {
                id: 1,
                parent: 0,
                role: 6,
                actions: 1,
                flags: 0,
                name: "First",
                value: "",
                live: 0,
            },
            V5Node {
                id: 2,
                parent: 0,
                role: 6,
                actions: 1,
                flags: 0,
                name: "Second",
                value: "",
                live: 0,
            },
        ]);
        let after = v5_packet_with(&[
            root(),
            V5Node {
                id: 1,
                parent: 0,
                role: 6,
                actions: 1,
                flags: 0,
                name: "First",
                value: "",
                live: 0,
            },
            V5Node {
                id: 2,
                parent: 0,
                role: 6,
                actions: 1,
                flags: 4,
                name: "Second",
                value: "",
                live: 0,
            },
        ]);
        let update = diff_a11y(&parse_a11y(&before), &parse_a11y(&after)).expect("update");
        assert_eq!(update.focus, NodeId(3));
        let ids: Vec<NodeId> = update.nodes.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, vec![NodeId(1), NodeId(3)]);
    }

    #[test]
    fn live_region_value_change_includes_node() {
        let before = v5_packet_with(&[
            root(),
            V5Node {
                id: 1,
                parent: 0,
                role: 22,
                actions: 0,
                flags: 0,
                name: "Status",
                value: "idle",
                live: 2,
            },
        ]);
        let after = v5_packet_with(&[
            root(),
            V5Node {
                id: 1,
                parent: 0,
                role: 22,
                actions: 0,
                flags: 0,
                name: "Status",
                value: "busy",
                live: 2,
            },
        ]);
        let update = diff_a11y(&parse_a11y(&before), &parse_a11y(&after)).expect("update");
        let status = node_by_id(&update, 2);
        assert_eq!(status.value(), Some("busy"));
        assert_eq!(status.live(), Some(Live::Assertive));
    }

    #[test]
    fn removed_node_rebuilds_former_parent() {
        let before = v5_packet_with(&[
            root(),
            V5Node {
                id: 1,
                parent: 0,
                role: 1,
                actions: 0,
                flags: 0,
                name: "Group",
                value: "",
                live: 0,
            },
            V5Node {
                id: 2,
                parent: 1,
                role: 6,
                actions: 1,
                flags: 0,
                name: "Row",
                value: "",
                live: 0,
            },
        ]);
        let after = v5_packet_with(&[
            root(),
            V5Node {
                id: 1,
                parent: 0,
                role: 1,
                actions: 0,
                flags: 0,
                name: "Group",
                value: "",
                live: 0,
            },
        ]);
        let update = diff_a11y(&parse_a11y(&before), &parse_a11y(&after)).expect("update");
        assert!(node_by_id(&update, 2).children().is_empty());
        assert!(!update.nodes.iter().any(|(id, _)| *id == NodeId(3)));
    }

    #[test]
    fn update_nodes_are_in_ascending_id_order() {
        let before = v5_packet_with(&[
            root(),
            V5Node {
                id: 1,
                parent: 0,
                role: 6,
                actions: 1,
                flags: 0,
                name: "One",
                value: "a",
                live: 0,
            },
            V5Node {
                id: 2,
                parent: 0,
                role: 6,
                actions: 1,
                flags: 0,
                name: "Two",
                value: "b",
                live: 0,
            },
            V5Node {
                id: 3,
                parent: 0,
                role: 6,
                actions: 1,
                flags: 0,
                name: "Three",
                value: "c",
                live: 0,
            },
        ]);
        let reordered = v5_packet_with(&[
            root(),
            V5Node {
                id: 3,
                parent: 0,
                role: 6,
                actions: 1,
                flags: 0,
                name: "Three",
                value: "c",
                live: 0,
            },
            V5Node {
                id: 1,
                parent: 0,
                role: 6,
                actions: 1,
                flags: 0,
                name: "One",
                value: "a",
                live: 0,
            },
            V5Node {
                id: 2,
                parent: 0,
                role: 6,
                actions: 1,
                flags: 0,
                name: "Two",
                value: "b",
                live: 0,
            },
        ]);
        let prev = parse_a11y(&before);
        assert!(diff_a11y(&prev, &parse_a11y(&reordered)).is_none());

        let after = v5_packet_with(&[
            root(),
            V5Node {
                id: 3,
                parent: 0,
                role: 6,
                actions: 1,
                flags: 0,
                name: "Three",
                value: "C",
                live: 0,
            },
            V5Node {
                id: 1,
                parent: 0,
                role: 6,
                actions: 1,
                flags: 0,
                name: "One",
                value: "A",
                live: 0,
            },
            V5Node {
                id: 2,
                parent: 0,
                role: 6,
                actions: 1,
                flags: 0,
                name: "Two",
                value: "B",
                live: 0,
            },
        ]);
        let update = diff_a11y(&prev, &parse_a11y(&after)).expect("update");
        let ids: Vec<u64> = update.nodes.iter().map(|(id, _)| id.0).collect();
        assert_eq!(ids, vec![1, 2, 3, 4]);
    }

    #[test]
    fn v4_packet_builds_accesskit_tree() {
        let records = parse_a11y(&v4_packet());
        assert_eq!(records.len(), 2);
        assert_eq!(records[1].expanded, -1);
        assert_eq!(records[1].description, "");
        let update = build_a11y(&records).expect("tree");
        assert_eq!(update.nodes.len(), 2);
        let button = update
            .nodes
            .iter()
            .find(|(id, _)| *id == NodeId(2))
            .map(|(_, n)| n)
            .expect("button");
        assert_eq!(button.role(), Role::Button);
        assert_eq!(button.label(), Some("Add"));
        assert!(button.supports_action(Action::Click));
        assert!(button.supports_action(Action::Focus));
        assert_eq!(button.is_selected(), None);
        assert_eq!(button.is_expanded(), None);
        let root = update
            .nodes
            .iter()
            .find(|(id, _)| *id == NodeId(1))
            .map(|(_, n)| n)
            .expect("root");
        assert!(root.children().contains(&NodeId(2)));
    }

    #[test]
    fn v5_packet_maps_extended_state() {
        let records = parse_a11y(&v5_packet());
        assert_eq!(records.len(), 2);
        assert_eq!(records[1].name, "Save");
        assert_eq!(records[1].value, "0.5");
        assert_eq!(records[1].description, "primary action");
        assert_eq!(records[1].value_min, 0.0);
        assert_eq!(records[1].value_max, 10.0);
        assert_eq!(records[1].value_step, 0.5);
        let update = build_a11y(&records).expect("tree");
        let button = update
            .nodes
            .iter()
            .find(|(id, _)| *id == NodeId(2))
            .map(|(_, n)| n)
            .expect("button");
        assert_eq!(button.role(), Role::Button);
        assert_eq!(button.label(), Some("Save"));
        assert_eq!(button.value(), Some("0.5"));
        assert_eq!(button.is_selected(), Some(true));
        assert_eq!(button.is_expanded(), Some(true));
        assert!(button.is_busy());
        assert!(button.is_required());
        assert_eq!(button.invalid(), Some(Invalid::Spelling));
        assert_eq!(button.level(), Some(2));
        assert_eq!(button.live(), Some(Live::Assertive));
        assert_eq!(button.numeric_value(), Some(0.5));
        assert_eq!(button.min_numeric_value(), Some(0.0));
        assert_eq!(button.max_numeric_value(), Some(10.0));
        assert_eq!(button.numeric_value_step(), Some(0.5));
        assert_eq!(button.description(), Some("primary action"));
        assert!(button.supports_action(Action::Click));
        assert!(button.supports_action(Action::Focus));
        let root = update
            .nodes
            .iter()
            .find(|(id, _)| *id == NodeId(1))
            .map(|(_, n)| n)
            .expect("root");
        assert_eq!(root.is_expanded(), Some(false));
    }

    #[test]
    fn v6_relations_round_trip_into_tree() {
        let packet = v6_packet_with(&[
            v6_node(0, -1, 1),
            V6Node {
                name: "Name",
                ..v6_node(1, 0, 4)
            },
            V6Node {
                name: "Help",
                ..v6_node(2, 0, 4)
            },
            V6Node {
                name: "Field",
                label_id: 1,
                described_by_id: 2,
                controls_id: 1,
                ..v6_node(3, 0, 11)
            },
        ]);
        let records = parse_a11y(&packet);
        assert_eq!(records.len(), 4);
        assert_eq!(records[3].label_id, 1);
        assert_eq!(records[3].described_by_id, 2);
        assert_eq!(records[3].controls_id, 1);
        assert_eq!(records[3].announce_seq, 0);
        let update = build_a11y(&records).expect("tree");
        let field = node_by_id(&update, 4);
        assert_eq!(field.labelled_by(), &[NodeId(2)]);
        assert_eq!(field.described_by(), &[NodeId(3)]);
        assert_eq!(field.controls(), &[NodeId(2)]);
    }

    #[test]
    fn v6_live_region_announcement_sequence_drives_update() {
        let before = v6_packet_with(&[
            v6_node(0, -1, 1),
            V6Node {
                name: "Status",
                value: "idle",
                live: 2,
                ..v6_node(1, 0, 22)
            },
        ]);
        let posted = v6_packet_with(&[
            v6_node(0, -1, 1),
            V6Node {
                name: "Status",
                value: "idle",
                live: 2,
                announce_seq: 1,
                ..v6_node(1, 0, 22)
            },
        ]);
        let prev = parse_a11y(&before);
        assert!(diff_a11y(&prev, &parse_a11y(&before)).is_none());
        let update = diff_a11y(&prev, &parse_a11y(&posted)).expect("update");
        let status = node_by_id(&update, 2);
        assert_eq!(status.value(), Some("idle"));
        assert_eq!(status.live(), Some(Live::Assertive));
    }

    #[test]
    fn v6_relation_only_change_produces_single_node_update() {
        let before = v6_packet_with(&[v6_node(0, -1, 1), v6_node(1, 0, 6)]);
        let after = v6_packet_with(&[
            V6Node {
                controls_id: 1,
                ..v6_node(0, -1, 1)
            },
            v6_node(1, 0, 6),
        ]);
        let update = diff_a11y(&parse_a11y(&before), &parse_a11y(&after)).expect("update");
        assert_eq!(update.nodes.len(), 1);
        let root = node_by_id(&update, 1);
        assert_eq!(root.controls(), &[NodeId(2)]);
    }

    #[test]
    fn legacy_v5_and_v4_packets_still_parse() {
        let v5 = parse_a11y(&v5_packet());
        assert_eq!(v5.len(), 2);
        assert_eq!(v5[1].label_id, 0);
        assert_eq!(v5[1].described_by_id, 0);
        assert_eq!(v5[1].controls_id, 0);
        assert_eq!(v5[1].announce_seq, 0);

        let v4 = parse_a11y(&v4_packet());
        assert_eq!(v4.len(), 2);
        assert_eq!(v4[1].name, "Add");
        assert_eq!(v4[1].label_id, 0);
        assert_eq!(v4[1].described_by_id, 0);
        assert_eq!(v4[1].controls_id, 0);
        assert_eq!(v4[1].announce_seq, 0);
    }

    #[test]
    fn unknown_version_with_v6_stride_fails_closed() {
        let mut packet = v6_packet_with(&[v6_node(0, -1, 1), v6_node(1, 0, 6)]);
        packet[4..8].copy_from_slice(&9u32.to_le_bytes());
        assert!(parse_a11y(&packet).is_empty());
        assert!(build_a11y(&parse_a11y(&packet)).is_none());

        let mut v5_version_v6_stride = v6_packet_with(&[v6_node(0, -1, 1)]);
        v5_version_v6_stride[4..8].copy_from_slice(&A11Y_VERSION_V5.to_le_bytes());
        assert!(parse_a11y(&v5_version_v6_stride).is_empty());
    }

    #[test]
    fn unknown_version_and_short_packets_fail_closed() {
        let mut wrong_version = v5_packet();
        wrong_version[4..8].copy_from_slice(&9u32.to_le_bytes());
        assert!(parse_a11y(&wrong_version).is_empty());
        assert!(build_a11y(&parse_a11y(&wrong_version)).is_none());

        let mut wrong_magic = v5_packet();
        wrong_magic[0..4].copy_from_slice(&7u32.to_le_bytes());
        assert!(parse_a11y(&wrong_magic).is_empty());

        assert!(parse_a11y(&v5_packet()[..A11Y_HEADER_V5 + 4]).is_empty());
    }
}

#[cfg(test)]
mod frame_tests {
    use super::*;

    fn header(count: u32, width: u32, height: u32, scale: u32, color: u32) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&424242u32.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&64u32.to_le_bytes());
        out.extend_from_slice(&width.to_le_bytes());
        out.extend_from_slice(&height.to_le_bytes());
        out.extend_from_slice(&scale.to_le_bytes());
        out.extend_from_slice(&color.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&(48 + count * 64).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out
    }

    #[test]
    fn frame_packet_reads_tags() {
        let data = header(7, 940, 780, 2000, 1);
        let parsed = parse_frame_packet(&data).expect("packet");
        assert_eq!(parsed.count, 7);
        assert_eq!(parsed.width, 940);
        assert_eq!(parsed.height, 780);
        assert_eq!(parsed.scale, 2000);
        assert_eq!(parsed.color, 1);
    }

    #[test]
    fn frame_packet_rejects_bad_magic_and_short_buffers() {
        let mut data = header(1, 10, 10, 1000, 0);
        data[0] = 0;
        assert!(parse_frame_packet(&data).is_none());
        assert!(parse_frame_packet(&data[..16]).is_none());
    }
}

#[cfg(test)]
mod window_tests {
    use super::pick_event_window;

    #[test]
    fn routing_prefers_primary_then_first_child() {
        assert_eq!(pick_event_window(true, &[true, true]), Some(0));
        assert_eq!(pick_event_window(false, &[false, true, true]), Some(2));
        assert_eq!(pick_event_window(false, &[false, false]), None);
        assert_eq!(pick_event_window(true, &[]), Some(0));
    }
}

#[cfg(test)]
mod input_tests {
    use super::{key_text_bytes, scroll_pixels};

    #[test]
    fn control_keys_do_not_type_text() {
        assert!(key_text_bytes("\u{8}").is_empty());
        assert!(key_text_bytes("\r").is_empty());
        assert!(key_text_bytes("\t").is_empty());
        assert_eq!(key_text_bytes("a"), b"a");
        assert_eq!(key_text_bytes("é"), "é".as_bytes());
    }

    #[test]
    fn scroll_keeps_subpixel_remainders() {
        let mut acc = 0.0;
        assert_eq!(scroll_pixels(&mut acc, 10.0), 40);
        assert_eq!(scroll_pixels(&mut acc, 0.25), 1);
        let mut small = 0.0;
        assert_eq!(scroll_pixels(&mut small, 0.1), 0);
        assert_eq!(scroll_pixels(&mut small, 0.1), 0);
        assert_eq!(scroll_pixels(&mut small, 0.1), 1);
    }
}

#[cfg(test)]
mod touch_tests {
    use std::collections::{BTreeMap, VecDeque};

    use super::{cancel_touches, release_touch, Event, EventQueueView};

    struct Queue {
        events: VecDeque<Event>,
        text_buf: Vec<u8>,
        clipboard_gen: u32,
        last_a: i32,
        last_b: i32,
        last_c: i32,
    }

    impl Queue {
        fn new(events: Vec<Event>) -> Self {
            Self {
                events: events.into(),
                text_buf: Vec::new(),
                clipboard_gen: 0,
                last_a: 0,
                last_b: 0,
                last_c: 0,
            }
        }

        fn next_event(&mut self) -> Option<(i32, i32, i32, i32)> {
            let code = {
                let mut view = EventQueueView {
                    events: &mut self.events,
                    text_buf: &mut self.text_buf,
                    clipboard_gen: self.clipboard_gen,
                    last_a: &mut self.last_a,
                    last_b: &mut self.last_b,
                    last_c: &mut self.last_c,
                };
                view.next_event()?
            };
            Some((code, self.last_a, self.last_b, self.last_c))
        }
    }

    #[test]
    fn touch_events_encode_codes_and_coordinates() {
        let mut queue = Queue::new(vec![
            Event::TouchDown(3, 120, 240),
            Event::TouchMove(3, 130, 250),
            Event::TouchUp(3, 140, 260),
            Event::TouchCancel(4),
        ]);
        assert_eq!(queue.next_event(), Some((25, 3, 120, 240)));
        assert_eq!(queue.next_event(), Some((26, 3, 130, 250)));
        assert_eq!(queue.next_event(), Some((27, 3, 140, 260)));
        assert_eq!(queue.next_event(), Some((28, 4, 0, 0)));
        assert_eq!(queue.next_event(), None);
    }

    #[test]
    fn touch_up_without_down_reports_safe_zero() {
        let mut touches = BTreeMap::new();
        touches.insert(1, (10.0, 20.0));
        let mut events = VecDeque::new();
        release_touch(&mut touches, &mut events, 2);
        assert_eq!(touches.len(), 1);
        let mut queue = Queue::new(events.into_iter().collect());
        assert_eq!(queue.next_event(), Some((27, 2, 0, 0)));
    }

    #[test]
    fn touch_cancel_clears_all_tracked_ids() {
        let mut touches = BTreeMap::new();
        touches.insert(1, (10.0, 20.0));
        touches.insert(2, (30.0, 40.0));
        let mut events = VecDeque::new();
        cancel_touches(&mut touches, &mut events);
        assert!(touches.is_empty());
        let mut queue = Queue::new(events.into_iter().collect());
        assert_eq!(queue.next_event(), Some((28, 1, 0, 0)));
        assert_eq!(queue.next_event(), Some((28, 2, 0, 0)));
        assert_eq!(queue.next_event(), None);
    }
}
