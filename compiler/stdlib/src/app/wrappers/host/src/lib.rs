use std::collections::VecDeque;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::slice;
use std::sync::{Mutex, OnceLock};

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, RawDisplayHandle, RawWindowHandle,
    WaylandDisplayHandle, WaylandWindowHandle,
};
use vello::peniko::Color;
use vello::{AaConfig, RenderParams, Renderer, RendererOptions};
use wayland_client::{
    delegate_noop,
    globals::{registry_queue_init, GlobalListContents},
    protocol::{wl_compositor, wl_keyboard, wl_pointer, wl_registry, wl_seat, wl_surface},
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum,
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
}

struct GpuCore {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Mutex<Renderer>,
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
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

struct Gpu {
    resources: GpuCore,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    target: wgpu::Texture,
    target_view: wgpu::TextureView,
    scratch: Option<(wgpu::Texture, wgpu::TextureView, u32, u32)>,
    blitter: wgpu::util::TextureBlitter,
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
    scroll_acc: f32,
    last_a: i32,
    last_b: i32,
    generation: u32,
}

struct Host {
    queue: EventQueue<State>,
    state: State,
}

impl State {
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
            .find(|f| {
                matches!(
                    f,
                    wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Bgra8Unorm
                )
            })
            .or_else(|| caps.formats.first().copied())
            .unwrap_or(wgpu::TextureFormat::Bgra8Unorm);
        let width = self.width.max(1);
        let height = self.height.max(1);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width,
            height,
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 1,
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
        let (target, target_view) = make_target(&resources.device, width, height);
        let blitter = wgpu::util::TextureBlitter::new(&resources.device, format);
        self.gpu = Some(Gpu {
            resources,
            surface,
            config,
            target,
            target_view,
            scratch: None,
            blitter,
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
        if std::env::var_os("RUNIX_TRACE").is_some() {
            eprintln!("target {}x{}", width, height);
        }
        gpu.config.width = width;
        gpu.config.height = height;
        gpu.surface.configure(&gpu.resources.device, &gpu.config);
        let (target, view) = make_target(&gpu.resources.device, width, height);
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
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return 1;
            }
            wgpu::CurrentSurfaceTexture::Validation => return -4,
        };

        if full {
            let scene = scene::to_gpu(&commands);
            let params = RenderParams {
                base_color: Color::TRANSPARENT,
                width: surface_w,
                height: surface_h,
                antialiasing_method: AaConfig::Area,
            };
            if gpu
                .resources
                .renderer
                .lock()
                .unwrap()
                .render_to_texture(
                    &gpu.resources.device,
                    &gpu.resources.queue,
                    &scene,
                    &gpu.target_view,
                    &params,
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
                let (texture, view) = make_target(&gpu.resources.device, max_w, max_h);
                gpu.scratch = Some((texture, view, max_w, max_h));
            }
            let (scratch, scratch_view, sw, sh) = gpu.scratch.as_ref().unwrap();
            let params = RenderParams {
                base_color: Color::TRANSPARENT,
                width: *sw,
                height: *sh,
                antialiasing_method: AaConfig::Area,
            };
            for rect in rects {
                let scene = scene::to_gpu_at(&commands, -(rect.0 as f64), -(rect.1 as f64));
                if gpu
                    .resources
                    .renderer
                    .lock()
                    .unwrap()
                    .render_to_texture(
                        &gpu.resources.device,
                        &gpu.resources.queue,
                        &scene,
                        scratch_view,
                        &params,
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

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu
            .resources
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        gpu.blitter
            .copy(&gpu.resources.device, &mut encoder, &gpu.target_view, &view);
        gpu.resources.queue.submit(Some(encoder.finish()));
        gpu.resources.queue.present(frame);
        if std::env::var_os("RUNIX_TRACE").is_some() {
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
        let Host { queue, state } = self;
        let _ = queue.blocking_dispatch(state);
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
            xdg_toplevel::Event::Configure { width, height, .. } => {
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
        }
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
                surface_x,
                surface_y,
                ..
            }
            | wl_pointer::Event::Motion {
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
                button,
                state: WEnum::Value(wl_pointer::ButtonState::Pressed),
                ..
            } if button == BTN_LEFT => {
                state.events.push_back(Event::PointerDown(
                    state.pointer_x as i32,
                    state.pointer_y as i32,
                ));
            }
            wl_pointer::Event::Button {
                button,
                state: WEnum::Value(wl_pointer::ButtonState::Released),
                ..
            } if button == BTN_LEFT => {
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
                state.scroll_acc += value as f32;
                while state.scroll_acc >= 10.0 {
                    state.scroll_acc -= 10.0;
                    state.events.push_back(Event::Scroll(1));
                }
                while state.scroll_acc <= -10.0 {
                    state.scroll_acc += 10.0;
                    state.events.push_back(Event::Scroll(-1));
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
        if let wl_keyboard::Event::Key {
            key,
            state: WEnum::Value(key_state),
            ..
        } = event
        {
            let code = key as i32;
            match key_state {
                wl_keyboard::KeyState::Pressed => state.events.push_back(Event::KeyDown(code)),
                wl_keyboard::KeyState::Released => state.events.push_back(Event::KeyUp(code)),
                _ => {}
            }
        }
    }
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
    toplevel.set_app_id("dev.runix.app".to_string());
    surface.commit();

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
            scroll_acc: 0.0,
            last_a: 0,
            last_b: 0,
            generation: 0,
        },
    };
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
        "runix app".to_string()
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
fn next_event(host: &mut Host) -> Option<i32> {
    loop {
        let event = host.state.events.pop_front()?;
        let (code, a, b) = match event {
            Event::Close => (1, 0, 0),
            Event::Resize(w, h) => (2, w, h),
            Event::PointerDown(x, y) => (3, x, y),
            Event::PointerUp(x, y) => (4, x, y),
            Event::PointerMove(x, y) => (5, x, y),
            Event::KeyDown(k) => (6, k, 0),
            Event::KeyUp(k) => (7, k, 0),
            Event::SurfaceLost => (8, 0, 0),
            Event::Scroll(d) => {
                if std::env::var_os("RUNIX_TRACE").is_some() {
                    eprintln!("scroll step={d}");
                }
                (9, d, 0)
            }
            Event::PointerLeave => (10, 0, 0),
        };
        host.state.last_a = a;
        host.state.last_b = b;
        return Some(code);
    }
}

#[no_mangle]
pub extern "C" fn host_pump(handle: *mut c_void) -> i32 {
    let Some(host) = (unsafe { (handle as *mut Host).as_mut() }) else {
        return -1;
    };
    loop {
        if let Some(code) = next_event(host) {
            return code;
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
pub extern "C" fn host_pump_timeout(handle: *mut c_void, block: i32) -> i32 {
    let Some(host) = (unsafe { (handle as *mut Host).as_mut() }) else {
        return -1;
    };
    if block != 0 {
        return host_pump(handle);
    }
    if let Some(code) = next_event(host) {
        return code;
    }
    if host.state.closed {
        return 1;
    }
    let Host { queue, state } = host;
    if let Some(guard) = queue.prepare_read() {
        let _ = guard.read();
    }
    let _ = queue.dispatch_pending(state);
    next_event(host).unwrap_or(0)
}

/// Wait until the previously presented buffer is free for reuse.
///
/// The Wayland swapchain already paces acquisition, so this is a no-op here.
/// An Epiclese host blocks on frame_done instead.
#[no_mangle]
pub extern "C" fn host_wait(handle: *mut c_void) -> i32 {
    if unsafe { (handle as *mut Host).as_ref() }.is_none() {
        return -1;
    }
    0
}

#[no_mangle]
pub extern "C" fn host_event_a(handle: *mut c_void) -> i32 {
    unsafe { (handle as *mut Host).as_ref() }
        .map(|h| h.state.last_a)
        .unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn host_event_b(handle: *mut c_void) -> i32 {
    unsafe { (handle as *mut Host).as_ref() }
        .map(|h| h.state.last_b)
        .unwrap_or(0)
}

/// Changes whenever the host recreates its target texture. The app forces a
/// full frame when this differs from the last value it saw.
#[no_mangle]
pub extern "C" fn host_generation(handle: *mut c_void) -> i32 {
    unsafe { (handle as *mut Host).as_ref() }
        .map(|h| h.state.generation as i32)
        .unwrap_or(-1)
}

#[no_mangle]
pub extern "C" fn host_width(handle: *mut c_void) -> i32 {
    unsafe { (handle as *mut Host).as_ref() }
        .map(|h| h.state.width as i32)
        .unwrap_or(-1)
}

#[no_mangle]
pub extern "C" fn host_height(handle: *mut c_void) -> i32 {
    unsafe { (handle as *mut Host).as_ref() }
        .map(|h| h.state.height as i32)
        .unwrap_or(-1)
}

#[no_mangle]
pub extern "C" fn host_present(
    handle: *mut c_void,
    list: *const u8,
    len: i32,
    damage: *const i32,
    count: i32,
) -> i32 {
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
    host.state.present(bytes, &rects)
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
pub extern "C" fn host_abi_version() -> i32 {
    1
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
) -> i32 {
    if w <= 0 || h <= 0 || list.is_null() || out.is_null() || len < 0 {
        return -1;
    }
    let bytes = unsafe { slice::from_raw_parts(list, len as usize) };
    let scene = match scene::from_list(bytes) {
        Ok(scene) => scene,
        Err(code) => return code,
    };
    let Some(core) = core(0) else {
        return -3;
    };
    let (texture, view) = make_target(&core.device, w as u32, h as u32);
    let params = RenderParams {
        base_color: Color::TRANSPARENT,
        width: w as u32,
        height: h as u32,
        antialiasing_method: AaConfig::Area,
    };
    if core
        .renderer
        .lock()
        .unwrap()
        .render_to_texture(&core.device, &core.queue, &scene, &view, &params)
        .is_err()
    {
        return -5;
    }
    readback(&core.device, &core.queue, &texture, w as u32, h as u32, out)
}
