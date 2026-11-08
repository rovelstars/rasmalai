use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::slice;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use scene::Command;
use vello_cpu::filter_effects::{Filter, FilterFunction};
use vello_cpu::kurbo::{Affine, BezPath, Cap, Join, Point, Rect, RoundedRect, Shape, Stroke};
use vello_cpu::peniko::color::DynamicColor;
use vello_cpu::peniko::{
    BlendMode, Color, ColorStop, ColorStops, Compose, Extend, Fill, Gradient, GradientKind,
    ImageData, LinearGradientPosition, Mix, RadialGradientPosition,
};
use vello_cpu::PaintType;
use vello_cpu::{Glyph, Image, ImageSource, Mask, Pixmap, RenderContext, Resources};

const IMAGE_CACHE_MAX_ENTRIES: usize = 64;
const IMAGE_CACHE_MAX_BYTES: usize = 32 * 1024 * 1024;

const MASK_CACHE_MAX_ENTRIES: usize = 32;
const MASK_CACHE_MAX_BYTES: usize = 32 * 1024 * 1024;
const MASK_SUPERSAMPLE: u32 = 4;

struct CachedImage {
    source: ImageSource,
    width: u32,
    height: u32,
    bytes: usize,
}

struct ImageCache {
    entries: HashMap<ImageKey, CachedImage>,
    order: VecDeque<ImageKey>,
    bytes: usize,
    hits: u64,
    misses: u64,
}

impl ImageCache {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            bytes: 0,
            hits: 0,
            misses: 0,
        }
    }

    fn evict_oldest(&mut self) {
        if let Some(key) = self.order.pop_front() {
            if let Some(entry) = self.entries.remove(&key) {
                self.bytes -= entry.bytes;
            }
        }
    }

    fn insert(
        &mut self,
        key: ImageKey,
        source: ImageSource,
        width: u32,
        height: u32,
        bytes: usize,
    ) {
        if bytes > IMAGE_CACHE_MAX_BYTES {
            return;
        }
        while self.entries.len() >= IMAGE_CACHE_MAX_ENTRIES {
            self.evict_oldest();
        }
        while self.bytes + bytes > IMAGE_CACHE_MAX_BYTES {
            self.evict_oldest();
        }
        self.bytes += bytes;
        self.order.push_back(key);
        self.entries.insert(
            key,
            CachedImage {
                source,
                width,
                height,
                bytes,
            },
        );
    }
}

thread_local! {
    static IMAGE_CACHE: RefCell<ImageCache> = RefCell::new(ImageCache::new());
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

type ImageKey = (u64, usize, u32, u32);

const DECODE_READY_MAX_ENTRIES: usize = 16;
const DECODE_READY_MAX_BYTES: usize = 32 * 1024 * 1024;
const DECODE_QUEUE_MAX: usize = 64;
const DECODE_FAILED_MAX: usize = 128;

struct DecodeRequest {
    key: ImageKey,
    bytes: Vec<u8>,
    target_w: u32,
    target_h: u32,
}

struct ReadyImage {
    image: ImageData,
    bytes: usize,
}

struct DecodeState {
    queue: VecDeque<DecodeRequest>,
    active: HashSet<ImageKey>,
    pending: usize,
    ready: HashMap<ImageKey, ReadyImage>,
    order: VecDeque<ImageKey>,
    failed: HashSet<ImageKey>,
    failed_order: VecDeque<ImageKey>,
    ready_bytes: usize,
    #[cfg(test)]
    enqueued: u64,
}

impl DecodeState {
    fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            active: HashSet::new(),
            pending: 0,
            ready: HashMap::new(),
            order: VecDeque::new(),
            failed: HashSet::new(),
            failed_order: VecDeque::new(),
            ready_bytes: 0,
            #[cfg(test)]
            enqueued: 0,
        }
    }

    fn evict_ready(&mut self) {
        if let Some(key) = self.order.pop_front() {
            if let Some(entry) = self.ready.remove(&key) {
                self.ready_bytes = self.ready_bytes.saturating_sub(entry.bytes);
            }
        }
    }

    fn store_ready(&mut self, key: ImageKey, image: ImageData) {
        let bytes = image.data.data().len();
        while self.ready.len() >= DECODE_READY_MAX_ENTRIES {
            self.evict_ready();
        }
        // A single image larger than the byte budget is kept as the only entry
        // so that it still reaches the render thread instead of being re-decoded.
        while self.ready_bytes + bytes > DECODE_READY_MAX_BYTES && !self.ready.is_empty() {
            self.evict_ready();
        }
        self.ready_bytes += bytes;
        self.order.push_back(key);
        self.ready.insert(key, ReadyImage { image, bytes });
    }
}

fn decode_state() -> &'static Mutex<DecodeState> {
    static STATE: OnceLock<Mutex<DecodeState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(DecodeState::new()))
}

fn lock_decode_state() -> std::sync::MutexGuard<'static, DecodeState> {
    decode_state()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

fn decode_signal() -> &'static std::sync::Condvar {
    static SIGNAL: OnceLock<std::sync::Condvar> = OnceLock::new();
    SIGNAL.get_or_init(std::sync::Condvar::new)
}

fn ensure_decode_worker() {
    static WORKER: OnceLock<()> = OnceLock::new();
    WORKER.get_or_init(|| {
        let _ = thread::Builder::new()
            .name("cpu-image-decode".to_string())
            .spawn(decode_worker);
    });
}

fn decode_worker() {
    loop {
        let request = {
            let mut state = lock_decode_state();
            loop {
                if let Some(request) = state.queue.pop_front() {
                    break request;
                }
                state = decode_signal()
                    .wait(state)
                    .unwrap_or_else(|error| error.into_inner());
            }
        };
        let decoded =
            scene::decode_image_at(&request.bytes, request.target_w, request.target_h);
        let mut state = lock_decode_state();
        state.active.remove(&request.key);
        state.pending = state.pending.saturating_sub(1);
        match decoded {
            Some(image) => state.store_ready(request.key, image),
            None => {
                if state.failed.insert(request.key) {
                    state.failed_order.push_back(request.key);
                    while state.failed_order.len() > DECODE_FAILED_MAX {
                        if let Some(old) = state.failed_order.pop_front() {
                            state.failed.remove(&old);
                        }
                    }
                }
            }
        }
        drop(state);
        decode_signal().notify_all();
    }
}

fn request_image(key: ImageKey, png: &[u8], target_w: u32, target_h: u32) {
    ensure_decode_worker();
    let mut state = lock_decode_state();
    if state.active.contains(&key) || state.ready.contains_key(&key) || state.failed.contains(&key)
    {
        return;
    }
    if state.queue.len() >= DECODE_QUEUE_MAX {
        return;
    }
    state.active.insert(key);
    state.pending += 1;
    #[cfg(test)]
    {
        state.enqueued += 1;
    }
    state.queue.push_back(DecodeRequest {
        key,
        bytes: png.to_vec(),
        target_w,
        target_h,
    });
    drop(state);
    decode_signal().notify_one();
}

fn ready_image(key: ImageKey) -> Option<ImageData> {
    let state = lock_decode_state();
    state.ready.get(&key).map(|entry| entry.image.clone())
}

fn image_cache_lookup(key: ImageKey) -> Option<(ImageSource, u32, u32)> {
    IMAGE_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let found = cache
            .entries
            .get(&key)
            .map(|entry| (entry.source.clone(), entry.width, entry.height));
        if found.is_some() {
            cache.hits += 1;
        } else {
            cache.misses += 1;
        }
        found
    })
}

fn image_cache_store(key: ImageKey, source: ImageSource, width: u32, height: u32, bytes: usize) {
    IMAGE_CACHE.with(|cache| cache.borrow_mut().insert(key, source, width, height, bytes));
}

fn paint_image(
    ctx: &mut RenderContext,
    transform: Affine,
    source: ImageSource,
    width: u32,
    height: u32,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) {
    let iw = width as f64;
    let ih = height as f64;
    if iw <= 0.0 || ih <= 0.0 {
        return;
    }
    ctx.set_transform(Affine::IDENTITY);
    ctx.set_paint(Image {
        image: source,
        sampler: Default::default(),
    });
    ctx.set_transform(Affine::IDENTITY);
    ctx.set_paint_transform(
        transform * Affine::translate((x, y)) * Affine::scale_non_uniform(w / iw, h / ih),
    );
    ctx.fill_rect(&Rect::new(x, y, x + w, y + h));
    ctx.reset_paint_transform();
}

fn target_span(value: f64) -> u32 {
    if !value.is_finite() || value <= 0.0 {
        return 0;
    }
    value.round().clamp(1.0, u16::MAX as f64) as u32
}

fn image_target(bytes: &[u8], w: f64, h: f64) -> (u32, u32) {
    if scene::is_svg(bytes) {
        (target_span(w), target_span(h))
    } else {
        (0, 0)
    }
}

#[cfg(test)]
fn reset_decode_state() {
    let mut state = lock_decode_state();
    state.queue.clear();
    state.active.clear();
    state.pending = 0;
    state.ready.clear();
    state.order.clear();
    state.failed.clear();
    state.failed_order.clear();
    state.ready_bytes = 0;
    state.enqueued = 0;
    drop(state);
    decode_signal().notify_all();
}

#[cfg(test)]
fn decode_enqueued() -> u64 {
    lock_decode_state().enqueued
}

#[cfg(test)]
fn reset_image_cache() {
    IMAGE_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.entries.clear();
        cache.order.clear();
        cache.bytes = 0;
        cache.hits = 0;
        cache.misses = 0;
    });
}

/// Return `(hits, misses, entries, bytes)` for this thread's image source cache.
pub fn image_cache_stats() -> (u64, u64, usize, usize) {
    IMAGE_CACHE.with(|cache| {
        let cache = cache.borrow();
        (cache.hits, cache.misses, cache.entries.len(), cache.bytes)
    })
}

#[derive(Clone, Copy)]
struct MaskSpec {
    surface_w: u16,
    surface_h: u16,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    radius: f64,
    transform: Affine,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct MaskKey {
    surface: (u16, u16),
    x: u64,
    y: u64,
    w: u64,
    h: u64,
    radius: u64,
    transform: [u64; 6],
}

impl MaskSpec {
    fn key(self) -> MaskKey {
        MaskKey {
            surface: (self.surface_w, self.surface_h),
            x: self.x.to_bits(),
            y: self.y.to_bits(),
            w: self.w.to_bits(),
            h: self.h.to_bits(),
            radius: self.radius.to_bits(),
            transform: self.transform.as_coeffs().map(f64::to_bits),
        }
    }
}

struct CachedMask {
    mask: Mask,
    bytes: usize,
}

struct MaskCache {
    entries: HashMap<MaskKey, CachedMask>,
    order: VecDeque<MaskKey>,
    bytes: usize,
    hits: u64,
    misses: u64,
}

impl MaskCache {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            bytes: 0,
            hits: 0,
            misses: 0,
        }
    }

    fn evict_oldest(&mut self) {
        if let Some(key) = self.order.pop_front() {
            if let Some(entry) = self.entries.remove(&key) {
                self.bytes -= entry.bytes;
            }
        }
    }

    fn insert(&mut self, key: MaskKey, mask: Mask, bytes: usize) {
        if bytes > MASK_CACHE_MAX_BYTES {
            return;
        }
        while self.entries.len() >= MASK_CACHE_MAX_ENTRIES {
            self.evict_oldest();
        }
        while self.bytes + bytes > MASK_CACHE_MAX_BYTES {
            self.evict_oldest();
        }
        self.bytes += bytes;
        self.order.push_back(key);
        self.entries.insert(key, CachedMask { mask, bytes });
    }
}

thread_local! {
    static MASK_CACHE: RefCell<MaskCache> = RefCell::new(MaskCache::new());
}

/// Return `(hits, misses, entries, bytes)` for this thread's mask cache.
pub fn mask_cache_stats() -> (u64, u64, usize, usize) {
    MASK_CACHE.with(|cache| {
        let cache = cache.borrow();
        (cache.hits, cache.misses, cache.entries.len(), cache.bytes)
    })
}

#[cfg(test)]
fn reset_mask_cache() {
    MASK_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.entries.clear();
        cache.order.clear();
        cache.bytes = 0;
        cache.hits = 0;
        cache.misses = 0;
    });
}

fn rounded_rect_covers(px: f64, py: f64, x: f64, y: f64, w: f64, h: f64, radius: f64) -> bool {
    if w <= 0.0 || h <= 0.0 {
        return false;
    }
    if px < x || py < y || px > x + w || py > y + h {
        return false;
    }
    let radius = if radius.is_finite() {
        radius.max(0.0).min(w / 2.0).min(h / 2.0)
    } else {
        0.0
    };
    let cx = px.clamp(x + radius, x + w - radius);
    let cy = py.clamp(y + radius, y + h - radius);
    let dx = px - cx;
    let dy = py - cy;
    dx * dx + dy * dy <= radius * radius
}

fn build_mask(spec: MaskSpec) -> Mask {
    let MaskSpec {
        surface_w,
        surface_h,
        x,
        y,
        w,
        h,
        radius,
        transform,
    } = spec;
    let width = surface_w as usize;
    let height = surface_h as usize;
    let mut data = vec![0u8; width * height];
    if w > 0.0 && h > 0.0 && width > 0 && height > 0 {
        let corners = [
            Point::new(x, y),
            Point::new(x + w, y),
            Point::new(x, y + h),
            Point::new(x + w, y + h),
        ];
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for corner in corners {
            let p = transform * corner;
            min_x = min_x.min(p.x);
            min_y = min_y.min(p.y);
            max_x = max_x.max(p.x);
            max_y = max_y.max(p.y);
        }
        if min_x.is_finite() && min_y.is_finite() && max_x.is_finite() && max_y.is_finite() {
            let x0 = (min_x.floor() - 1.0).max(0.0) as usize;
            let y0 = (min_y.floor() - 1.0).max(0.0) as usize;
            let x1 = ((max_x.ceil() + 1.0).max(0.0) as usize).min(width);
            let y1 = ((max_y.ceil() + 1.0).max(0.0) as usize).min(height);
            let inverse = transform.inverse();
            let samples = MASK_SUPERSAMPLE * MASK_SUPERSAMPLE;
            let step = 1.0 / MASK_SUPERSAMPLE as f64;
            for py in y0..y1 {
                for px in x0..x1 {
                    let mut covered = 0u32;
                    for sy in 0..MASK_SUPERSAMPLE {
                        for sx in 0..MASK_SUPERSAMPLE {
                            let dx = px as f64 + (sx as f64 + 0.5) * step;
                            let dy = py as f64 + (sy as f64 + 0.5) * step;
                            let local = inverse * Point::new(dx, dy);
                            if rounded_rect_covers(local.x, local.y, x, y, w, h, radius) {
                                covered += 1;
                            }
                        }
                    }
                    let alpha = (covered * 255 + samples / 2) / samples;
                    data[py * width + px] = alpha as u8;
                }
            }
        }
    }
    Mask::from_parts(data, surface_w, surface_h)
}

fn mask_for(spec: MaskSpec) -> Mask {
    let key = spec.key();
    let cached = MASK_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        match cache.entries.get(&key) {
            Some(entry) => {
                let mask = entry.mask.clone();
                cache.hits += 1;
                Some(mask)
            }
            None => {
                cache.misses += 1;
                None
            }
        }
    });
    if let Some(mask) = cached {
        return mask;
    }
    let mask = build_mask(spec);
    let bytes = spec.surface_w as usize * spec.surface_h as usize;
    MASK_CACHE.with(|cache| cache.borrow_mut().insert(key, mask.clone(), bytes));
    mask
}

fn color_of(v: u32) -> Color {
    Color::from_rgba8((v >> 24) as u8, (v >> 16) as u8, (v >> 8) as u8, v as u8)
}

fn cap_of(bits: u8) -> Cap {
    match bits & 3 {
        1 => Cap::Round,
        2 => Cap::Square,
        _ => Cap::Butt,
    }
}

fn join_of(bits: u8) -> Join {
    match bits & 3 {
        1 => Join::Round,
        2 => Join::Bevel,
        _ => Join::Miter,
    }
}

fn blend_of(id: u8) -> BlendMode {
    match id {
        1 => Mix::Multiply.into(),
        2 => Mix::Screen.into(),
        3 => Mix::Overlay.into(),
        4 => Mix::Darken.into(),
        5 => Mix::Lighten.into(),
        6 => Mix::Difference.into(),
        7 => Mix::Exclusion.into(),
        8 => BlendMode::new(Mix::Normal, Compose::Plus),
        _ => BlendMode::default(),
    }
}

fn linear_brush(top: u32, bottom: u32, x: f64, y: f64, h: f64) -> PaintType {
    let stops = [
        ColorStop {
            offset: 0.0,
            color: DynamicColor::from_alpha_color(color_of(top)),
        },
        ColorStop {
            offset: 1.0,
            color: DynamicColor::from_alpha_color(color_of(bottom)),
        },
    ];
    PaintType::Gradient(Gradient {
        kind: GradientKind::Linear(LinearGradientPosition {
            start: Point::new(x, y),
            end: Point::new(x, y + h),
        }),
        extend: Extend::Pad,
        stops: ColorStops::from(&stops[..]),
        ..Default::default()
    })
}

fn gradient_paint(
    kind: u8,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    radius: f64,
    stops: &[(u32, f32)],
) -> PaintType {
    let ramp: Vec<ColorStop> = stops
        .iter()
        .map(|&(color, offset)| ColorStop {
            offset,
            color: DynamicColor::from_alpha_color(color_of(color)),
        })
        .collect();
    let kind = match kind {
        1 => GradientKind::Linear(LinearGradientPosition {
            start: Point::new(x, y),
            end: Point::new(x + w, y),
        }),
        2 => {
            let fallback = if w.is_finite() && h.is_finite() {
                (w.min(h) / 2.0).max(0.0)
            } else {
                0.0
            };
            let radius = if radius > 0.0 { radius } else { fallback };
            let radius = radius.clamp(0.0, f32::MAX as f64) as f32;
            GradientKind::Radial(RadialGradientPosition::new(
                Point::new(x + w / 2.0, y + h / 2.0),
                radius,
            ))
        }
        _ => GradientKind::Linear(LinearGradientPosition {
            start: Point::new(x, y),
            end: Point::new(x, y + h),
        }),
    };
    PaintType::Gradient(Gradient {
        kind,
        extend: Extend::Pad,
        stops: ColorStops::from(&ramp[..]),
        ..Default::default()
    })
}

fn draw_text(
    ctx: &mut RenderContext,
    resources: &mut Resources,
    text: &str,
    x: f64,
    y: f64,
    size: f32,
    weight: i32,
    italic: bool,
    tracking: f64,
    family: i32,
    color: Color,
    transform: Affine,
) {
    for run in scene::shape_styled(text, size, weight, italic, tracking, family) {
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

#[derive(Clone, Copy)]
enum LayerState {
    Clip,
    Mask,
    Blur(usize),
}

fn pop_open_layers(ctx: &mut RenderContext, layers: &mut Vec<LayerState>) {
    while let Some(state) = layers.pop() {
        let count = match state {
            LayerState::Clip | LayerState::Mask => 1,
            LayerState::Blur(count) => count,
        };
        for _ in 0..count {
            ctx.pop_layer();
        }
    }
}

#[no_mangle]
pub extern "C" fn cpu_render(w: i32, h: i32, list: *const u8, len: i32, out: *mut u8) -> i64 {
    if w <= 0 || h <= 0 || w > u16::MAX as i32 || h > u16::MAX as i32 {
        return -1;
    }
    if list.is_null() || out.is_null() || len < 0 {
        return -1;
    }
    let bytes = unsafe { slice::from_raw_parts(list, len as usize) };
    let commands = match scene::parse(bytes) {
        Ok(commands) => commands,
        Err(code) => return code as i64,
    };
    let width = w as u16;
    let height = h as u16;
    let mut ctx = RenderContext::new(width, height);
    let mut resources = Resources::new();
    let mut transform = Affine::IDENTITY;
    let mut stack: Vec<Affine> = Vec::new();
    let mut layers: Vec<LayerState> = Vec::new();
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
            Command::RoundShadow {
                color,
                x,
                y,
                w,
                h,
                radius,
                std_dev,
            } => {
                ctx.set_transform(transform);
                ctx.set_paint(color_of(*color));
                ctx.fill_blurred_rounded_rect(
                    &Rect::new(*x, *y, x + w, y + h),
                    *radius as f32,
                    *std_dev as f32,
                    false,
                );
            }
            Command::RoundRectGradient {
                top,
                bottom,
                x,
                y,
                w,
                h,
                radius,
            } => {
                ctx.set_transform(transform);
                ctx.set_paint(linear_brush(*top, *bottom, *x, *y, *h));
                let path = RoundedRect::new(*x, *y, x + w, y + h, *radius).to_path(0.1);
                ctx.fill_path(&path);
            }
            Command::GradientRect {
                x,
                y,
                w,
                h,
                kind,
                radius,
                stops,
            } => {
                ctx.set_transform(transform);
                ctx.set_paint(gradient_paint(*kind, *x, *y, *w, *h, *radius, stops));
                ctx.fill_rect(&Rect::new(*x, *y, x + w, y + h));
            }
            Command::Text {
                color,
                x,
                y,
                size,
                weight,
                italic,
                tracking,
                family,
                text,
            } => draw_text(
                &mut ctx,
                &mut resources,
                text,
                *x,
                *y,
                *size,
                *weight,
                *italic,
                *tracking,
                *family,
                color_of(*color),
                transform,
            ),
            Command::Image { x, y, w, h, png } => {
                if scene::is_raw_frame(png) {
                    if let Some(image) = scene::decode_image_at(png, 0, 0) {
                        let width = image.width;
                        let height = image.height;
                        let source = ImageSource::from_peniko_image_data(&image);
                        paint_image(&mut ctx, transform, source, width, height, *x, *y, *w, *h);
                    }
                } else {
                    let (target_w, target_h) = image_target(png, *w, *h);
                    let key = (fnv1a(png), png.len(), target_w, target_h);
                    if let Some((source, width, height)) = image_cache_lookup(key) {
                        paint_image(&mut ctx, transform, source, width, height, *x, *y, *w, *h);
                    } else if let Some(image) = ready_image(key) {
                        let width = image.width;
                        let height = image.height;
                        let bytes = image.data.data().len();
                        let source = ImageSource::from_peniko_image_data(&image);
                        image_cache_store(key, source.clone(), width, height, bytes);
                        paint_image(&mut ctx, transform, source, width, height, *x, *y, *w, *h);
                    } else {
                        request_image(key, png, target_w, target_h);
                    }
                }
            }
            Command::ClipPush { x, y, w, h } => {
                ctx.set_transform(transform);
                let path = Rect::new(*x, *y, x + w, y + h).to_path(0.1);
                ctx.push_clip_layer(&path);
                layers.push(LayerState::Clip);
            }
            Command::ClipPushRound { x, y, w, h, radius } => {
                ctx.set_transform(transform);
                let path = RoundedRect::new(*x, *y, x + w, y + h, *radius).to_path(0.1);
                ctx.push_clip_layer(&path);
                layers.push(LayerState::Clip);
            }
            Command::Polygon { color, points } => {
                if let Some((first, rest)) = points.split_first() {
                    let mut path = BezPath::new();
                    path.move_to(*first);
                    for p in rest {
                        path.line_to(*p);
                    }
                    path.close_path();
                    ctx.set_transform(transform);
                    ctx.set_paint(color_of(*color));
                    ctx.fill_path(&path);
                }
            }
            Command::PathFill {
                color,
                path,
                even_odd,
            } => {
                let rule = if *even_odd {
                    Fill::EvenOdd
                } else {
                    Fill::NonZero
                };
                ctx.set_transform(transform);
                ctx.set_fill_rule(rule);
                ctx.set_paint(color_of(*color));
                ctx.fill_path(path);
                ctx.set_fill_rule(Fill::NonZero);
            }
            Command::PathStroke {
                color,
                path,
                width,
                cap,
                join,
            } => {
                ctx.set_transform(transform);
                ctx.set_stroke(
                    Stroke::new(*width)
                        .with_caps(cap_of(*cap))
                        .with_join(join_of(*join)),
                );
                ctx.set_paint(color_of(*color));
                ctx.stroke_path(path);
            }
            Command::RoundRectStroke {
                color,
                x,
                y,
                w,
                h,
                radius,
                width,
            } => {
                ctx.set_transform(transform);
                ctx.set_stroke(Stroke::new(*width));
                ctx.set_paint(color_of(*color));
                let path = RoundedRect::new(*x, *y, x + w, y + h, *radius).to_path(0.1);
                ctx.stroke_path(&path);
            }
            Command::LayerPush {
                x,
                y,
                w,
                h,
                alpha,
                blend,
            } => {
                ctx.set_transform(transform);
                let clip = Rect::new(*x, *y, x + w, y + h).to_path(0.1);
                ctx.push_layer(
                    Some(&clip),
                    Some(blend_of(*blend)),
                    Some(*alpha),
                    None,
                    None,
                );
                layers.push(LayerState::Clip);
            }
            Command::ClipPop | Command::LayerPop => {
                if matches!(layers.last(), Some(LayerState::Clip)) {
                    layers.pop();
                    ctx.pop_layer();
                }
            }
            Command::PushMask { x, y, w, h, radius } => {
                ctx.set_transform(transform);
                let mask = mask_for(MaskSpec {
                    surface_w: width,
                    surface_h: height,
                    x: *x,
                    y: *y,
                    w: *w,
                    h: *h,
                    radius: *radius,
                    transform,
                });
                ctx.push_mask_layer(mask);
                layers.push(LayerState::Mask);
            }
            Command::PopMask => {
                if matches!(layers.last(), Some(LayerState::Mask)) {
                    layers.pop();
                    ctx.pop_layer();
                }
            }
            Command::PushBlur { x, y, w, h, sigma } => {
                let blur = *sigma > 0.0 && sigma.is_finite() && !ctx.is_multi_threaded();
                if blur {
                    ctx.set_transform(transform);
                    let clip = Rect::new(*x, *y, x + w, y + h).to_path(0.1);
                    ctx.push_clip_layer(&clip);
                    ctx.push_filter_layer(Filter::from_function(FilterFunction::Blur {
                        radius: *sigma as f32,
                    }));
                    layers.push(LayerState::Blur(2));
                } else {
                    layers.push(LayerState::Blur(0));
                }
            }
            Command::PopBlur => {
                if let Some(LayerState::Blur(count)) = layers.last().copied() {
                    layers.pop();
                    for _ in 0..count {
                        ctx.pop_layer();
                    }
                }
            }
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
    pop_open_layers(&mut ctx, &mut layers);
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
pub extern "C" fn cpu_abi_version() -> i64 {
    1
}

#[no_mangle]
pub extern "C" fn cpu_register_font(
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
pub extern "C" fn cpu_image_pending() -> i64 {
    lock_decode_state().pending as i64
}

#[no_mangle]
pub extern "C" fn cpu_wait_images(timeout_ms: i64) -> i64 {
    let deadline = Instant::now().checked_add(Duration::from_millis(timeout_ms.max(0) as u64));
    loop {
        let pending = lock_decode_state().pending;
        if pending == 0 {
            return 0;
        }
        let now = Instant::now();
        let sleep = match deadline {
            Some(deadline) if now >= deadline => return pending as i64,
            Some(deadline) => (deadline - now).min(Duration::from_millis(1)),
            None => Duration::from_millis(1),
        };
        thread::sleep(sleep);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put_u32(out: &mut [u8], at: usize, v: u32) {
        out[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn put_f64(out: &mut [u8], at: usize, v: f64) {
        out[at..at + 8].copy_from_slice(&v.to_le_bytes());
    }

    fn put_f32(out: &mut [u8], at: usize, v: f32) {
        out[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    #[test]
    fn clip_round_smoke() {
        let mut list = vec![0u8; 32 + 5 * 64];
        put_u32(&mut list, 0, 0x4458_4E52);
        put_u32(&mut list, 4, 3);
        put_u32(&mut list, 8, 5);
        put_u32(&mut list, 12, 64);
        put_u32(&mut list, 16, (32 + 5 * 64) as u32);
        put_u32(&mut list, 20, 0);
        put_u32(&mut list, 24, 100);
        put_u32(&mut list, 28, 100);

        let at = 32;
        put_u32(&mut list, at, 18);
        put_f64(&mut list, at + 16, 0.0);
        put_f64(&mut list, at + 24, 0.0);
        put_f64(&mut list, at + 32, 100.0);
        put_f64(&mut list, at + 40, 100.0);
        put_f64(&mut list, at + 48, 20.0);

        put_u32(&mut list, at + 64, 1);
        put_u32(&mut list, at + 68, 0xFF0000FF);
        put_f64(&mut list, at + 64 + 16, 0.0);
        put_f64(&mut list, at + 64 + 24, 0.0);
        put_f64(&mut list, at + 64 + 32, 100.0);
        put_f64(&mut list, at + 64 + 40, 100.0);

        put_u32(&mut list, at + 128, 3);
        put_f64(&mut list, at + 128 + 16, 10.0);
        put_f64(&mut list, at + 128 + 24, 10.0);
        put_f64(&mut list, at + 128 + 32, 80.0);
        put_f64(&mut list, at + 128 + 40, 80.0);

        put_u32(&mut list, at + 192, 4);
        put_u32(&mut list, at + 256, 4);

        let mut out = vec![0u8; 100 * 100 * 4];
        let rc = cpu_render(100, 100, list.as_ptr(), list.len() as i32, out.as_mut_ptr());
        assert_eq!(rc, 0);
    }

    #[test]
    fn path_fill_starts_at_the_move_point() {
        let mut list = vec![0u8; 32 + 64 + 131 + 175];
        put_u32(&mut list, 0, 0x4458_4E52);
        put_u32(&mut list, 4, 3);
        put_u32(&mut list, 8, 1);
        put_u32(&mut list, 12, 64);
        put_u32(&mut list, 16, 96);
        put_u32(&mut list, 20, 131 + 175);
        put_u32(&mut list, 24, 200);
        put_u32(&mut list, 28, 200);

        let at = 32;
        put_u32(&mut list, at, 11);
        put_u32(&mut list, at + 4, 0xFF0000FF);
        put_u32(&mut list, at + 8, 175);
        put_u32(&mut list, at + 12, 2);

        let blob = 96 + 175;
        list[blob] = 0;
        put_f64(&mut list, blob + 1, 20.0);
        put_f64(&mut list, blob + 9, 100.0);
        list[blob + 17] = 3;
        put_f64(&mut list, blob + 18, 60.0);
        put_f64(&mut list, blob + 26, 20.0);
        put_f64(&mut list, blob + 34, 100.0);
        put_f64(&mut list, blob + 42, 180.0);
        put_f64(&mut list, blob + 50, 140.0);
        put_f64(&mut list, blob + 58, 100.0);
        list[blob + 66] = 3;
        put_f64(&mut list, blob + 67, 140.0);
        put_f64(&mut list, blob + 75, 20.0);
        put_f64(&mut list, blob + 83, 180.0);
        put_f64(&mut list, blob + 91, 180.0);
        put_f64(&mut list, blob + 99, 220.0);
        put_f64(&mut list, blob + 107, 100.0);

        let mut out = vec![0u8; 200 * 200 * 4];
        let rc = cpu_render(200, 200, list.as_ptr(), list.len() as i32, out.as_mut_ptr());
        assert_eq!(rc, 0);
        let mut left = 0;
        for x in 20..55 {
            for y in 20..180 {
                let at = (y * 200 + x) * 4;
                if out[at + 3] > 0 {
                    left += 1;
                }
            }
        }
        assert!(left > 50, "fill ignores the move point: {left}");
    }

    const HEADER: usize = 32;
    const STRIDE: usize = 64;

    fn make_list(records: usize) -> Vec<u8> {
        let blob_at = HEADER + records * STRIDE;
        let mut out = vec![0u8; blob_at];
        put_u32(&mut out, 0, 0x4458_4E52);
        put_u32(&mut out, 4, 3);
        put_u32(&mut out, 8, records as u32);
        put_u32(&mut out, 12, STRIDE as u32);
        put_u32(&mut out, 16, blob_at as u32);
        put_u32(&mut out, 20, 0);
        out
    }

    fn record(out: &mut [u8], index: usize, kind: u32) -> usize {
        let at = HEADER + index * STRIDE;
        put_u32(out, at, kind);
        at
    }

    fn render(w: i32, h: i32, list: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8; (w * h * 4) as usize];
        let rc = cpu_render(w, h, list.as_ptr(), list.len() as i32, out.as_mut_ptr());
        assert_eq!(rc, 0);
        out
    }

    fn rgba(out: &[u8], w: usize, x: usize, y: usize) -> [u8; 4] {
        let at = (y * w + x) * 4;
        [out[at], out[at + 1], out[at + 2], out[at + 3]]
    }

    fn plain_red_list() -> Vec<u8> {
        let mut list = make_list(1);
        let at = record(&mut list, 0, 1);
        put_u32(&mut list, at + 4, 0xFF00_00FF);
        put_f64(&mut list, at + 16, 0.0);
        put_f64(&mut list, at + 24, 0.0);
        put_f64(&mut list, at + 32, 64.0);
        put_f64(&mut list, at + 40, 64.0);
        list
    }

    fn masked_red_list() -> Vec<u8> {
        let mut list = make_list(3);
        let at = record(&mut list, 0, 19);
        put_f64(&mut list, at + 16, 8.0);
        put_f64(&mut list, at + 24, 8.0);
        put_f64(&mut list, at + 32, 48.0);
        put_f64(&mut list, at + 40, 48.0);
        put_f64(&mut list, at + 48, 12.0);

        let at = record(&mut list, 1, 1);
        put_u32(&mut list, at + 4, 0xFF00_00FF);
        put_f64(&mut list, at + 16, 0.0);
        put_f64(&mut list, at + 24, 0.0);
        put_f64(&mut list, at + 32, 64.0);
        put_f64(&mut list, at + 40, 64.0);

        record(&mut list, 2, 20);
        list
    }

    #[test]
    fn mask_hides_pixels_outside_rounded_rect() {
        reset_mask_cache();
        let plain = render(64, 64, &plain_red_list());
        let masked = render(64, 64, &masked_red_list());

        assert_eq!(rgba(&plain, 64, 32, 32), rgba(&masked, 64, 32, 32));
        assert_eq!(rgba(&plain, 64, 2, 2)[3], 255);
        assert_eq!(rgba(&masked, 64, 2, 2)[3], 0);
        assert_eq!(rgba(&masked, 64, 9, 9)[3], 0);
    }

    #[test]
    fn blur_softens_a_hard_vertical_edge() {
        let mut list = make_list(4);
        let at = record(&mut list, 0, 21);
        put_f64(&mut list, at + 16, 0.0);
        put_f64(&mut list, at + 24, 0.0);
        put_f64(&mut list, at + 32, 64.0);
        put_f64(&mut list, at + 40, 64.0);
        put_f64(&mut list, at + 48, 4.0);

        let at = record(&mut list, 1, 1);
        put_u32(&mut list, at + 4, 0x0000_FFFF);
        put_f64(&mut list, at + 16, 0.0);
        put_f64(&mut list, at + 24, 0.0);
        put_f64(&mut list, at + 32, 32.0);
        put_f64(&mut list, at + 40, 64.0);

        let at = record(&mut list, 2, 1);
        put_u32(&mut list, at + 4, 0xFF00_00FF);
        put_f64(&mut list, at + 16, 32.0);
        put_f64(&mut list, at + 24, 0.0);
        put_f64(&mut list, at + 32, 32.0);
        put_f64(&mut list, at + 40, 64.0);

        record(&mut list, 3, 22);

        let out = render(64, 64, &list);
        let [r, g, b, a] = rgba(&out, 64, 32, 32);
        assert!(a > 0, "edge pixel is transparent");
        assert!(
            r > 10 && b > 10 && g < 40,
            "edge pixel is not a blend of both sides: {r},{g},{b}"
        );
    }

    #[test]
    fn unbalanced_pop_records_do_not_panic() {
        let mut list = make_list(5);
        record(&mut list, 0, 20);
        record(&mut list, 1, 22);
        let at = record(&mut list, 2, 1);
        put_u32(&mut list, at + 4, 0xFF00_00FF);
        put_f64(&mut list, at + 16, 0.0);
        put_f64(&mut list, at + 24, 0.0);
        put_f64(&mut list, at + 32, 32.0);
        put_f64(&mut list, at + 40, 32.0);
        record(&mut list, 3, 20);
        record(&mut list, 4, 22);

        let out = render(32, 32, &list);
        assert_eq!(rgba(&out, 32, 16, 16)[3], 255);
    }

    #[test]
    fn mask_cache_reuses_identical_parameters() {
        reset_mask_cache();
        assert_eq!(mask_cache_stats(), (0, 0, 0, 0));
        let list = masked_red_list();
        let _ = render(64, 64, &list);
        assert_eq!(mask_cache_stats(), (0, 1, 1, 64 * 64));
        let _ = render(64, 64, &list);
        assert_eq!(mask_cache_stats(), (1, 1, 1, 64 * 64));
    }

    #[test]
    fn styled_text_smoke() {
        let text = b"Bg";
        let blob_at = 32 + 64;
        let mut list = vec![0u8; blob_at + text.len()];
        put_u32(&mut list, 0, 0x4458_4E52);
        put_u32(&mut list, 4, 3);
        put_u32(&mut list, 8, 1);
        put_u32(&mut list, 12, 64);
        put_u32(&mut list, 16, blob_at as u32);
        put_u32(&mut list, 20, text.len() as u32);
        put_u32(&mut list, 24, 120);
        put_u32(&mut list, 28, 80);

        let at = 32;
        put_u32(&mut list, at, 2);
        put_u32(&mut list, at + 4, 0xFF0000FF);
        put_u32(&mut list, at + 8, 0);
        put_u32(&mut list, at + 12, text.len() as u32);
        put_f64(&mut list, at + 16, 4.0);
        put_f64(&mut list, at + 24, 8.0);
        put_f64(&mut list, at + 32, 32.0);
        put_f64(&mut list, at + 48, 10700.0);
        put_f64(&mut list, at + 56, 1.5);
        list[blob_at..].copy_from_slice(text);

        let mut out = vec![0u8; 120 * 80 * 4];
        let rc = cpu_render(120, 80, list.as_ptr(), list.len() as i32, out.as_mut_ptr());
        assert_eq!(rc, 0);
    }

    #[test]
    fn family_packed_text_smoke() {
        let text = b"Mono";
        let blob_at = 32 + 64;
        let mut list = vec![0u8; blob_at + text.len()];
        put_u32(&mut list, 0, 0x4458_4E52);
        put_u32(&mut list, 4, 3);
        put_u32(&mut list, 8, 1);
        put_u32(&mut list, 12, 64);
        put_u32(&mut list, 16, blob_at as u32);
        put_u32(&mut list, 20, text.len() as u32);
        put_u32(&mut list, 24, 120);
        put_u32(&mut list, 28, 80);

        let at = 32;
        put_u32(&mut list, at, 2);
        put_u32(&mut list, at + 4, 0xFF0000FF);
        put_u32(&mut list, at + 8, 0);
        put_u32(&mut list, at + 12, text.len() as u32);
        put_f64(&mut list, at + 16, 4.0);
        put_f64(&mut list, at + 24, 8.0);
        put_f64(&mut list, at + 32, 32.0);
        put_f64(&mut list, at + 48, 5_000_700.0);
        list[blob_at..].copy_from_slice(text);

        let mut out = vec![0u8; 120 * 80 * 4];
        let rc = cpu_render(120, 80, list.as_ptr(), list.len() as i32, out.as_mut_ptr());
        assert_eq!(rc, 0);
    }

    const INTER: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../../../../website/static/fonts/Inter-SemiBold.ttf"
    ));

    fn font_table(entries: &[(u32, &str)]) -> Vec<u8> {
        let mut body = Vec::new();
        for (id, name) in entries {
            body.extend_from_slice(&id.to_le_bytes());
            body.extend_from_slice(&(name.len() as u32).to_le_bytes());
            body.extend_from_slice(name.as_bytes());
        }
        let table_len = 12 + body.len();
        let mut out = Vec::with_capacity(table_len);
        out.extend_from_slice(&0x464E_5432u32.to_le_bytes());
        out.extend_from_slice(&(table_len as u32).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    #[test]
    fn named_font_table_shapes_and_paints() {
        assert_eq!(scene::register_font(100, "Inter SemiBold", INTER), 0);
        let table = font_table(&[(100, "Inter SemiBold")]);
        let text = b"Inter";
        let blob_at = HEADER + STRIDE;
        let mut list = vec![0u8; blob_at + table.len() + text.len()];
        put_u32(&mut list, 0, 0x4458_4E52);
        put_u32(&mut list, 4, 3);
        put_u32(&mut list, 8, 1);
        put_u32(&mut list, 12, STRIDE as u32);
        put_u32(&mut list, 16, blob_at as u32);
        put_u32(&mut list, 20, (table.len() + text.len()) as u32);
        put_u32(&mut list, 24, 200);
        put_u32(&mut list, 28, 120);

        let at = record(&mut list, 0, 2);
        put_u32(&mut list, at + 4, 0x0000_00FF);
        put_u32(&mut list, at + 8, table.len() as u32);
        put_u32(&mut list, at + 12, text.len() as u32);
        put_f64(&mut list, at + 16, 8.0);
        put_f64(&mut list, at + 24, 80.0);
        put_f64(&mut list, at + 32, 64.0);
        put_f64(&mut list, at + 48, 100_000_400.0);
        list[blob_at..blob_at + table.len()].copy_from_slice(&table);
        list[blob_at + table.len()..].copy_from_slice(text);

        let out = render(200, 120, &list);
        assert!(
            out.chunks_exact(4).any(|pixel| pixel[3] > 0),
            "named family text painted nothing"
        );
    }

    const TINY_PNG: [u8; 74] = [
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x06, 0x00, 0x00, 0x00, 0x72,
        0xB6, 0x0D, 0x24, 0x00, 0x00, 0x00, 0x11, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0x63, 0xF8,
        0xCF, 0xC0, 0xF0, 0x1F, 0x84, 0x19, 0x60, 0x0C, 0x00, 0x47, 0xCA, 0x07, 0xF9, 0x1A, 0xB6,
        0xF1, 0xA9, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    const TINY_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="2" height="2" fill="#0a141e"/></svg>"##;

    fn image_list(png: &[u8]) -> Vec<u8> {
        let blob_at = HEADER + STRIDE;
        let mut list = vec![0u8; blob_at + png.len()];
        put_u32(&mut list, 0, 0x4458_4E52);
        put_u32(&mut list, 4, 3);
        put_u32(&mut list, 8, 1);
        put_u32(&mut list, 12, STRIDE as u32);
        put_u32(&mut list, 16, blob_at as u32);
        put_u32(&mut list, 20, png.len() as u32);
        put_u32(&mut list, 24, 16);
        put_u32(&mut list, 28, 16);
        let at = record(&mut list, 0, 6);
        put_u32(&mut list, at + 8, 0);
        put_u32(&mut list, at + 12, png.len() as u32);
        put_f64(&mut list, at + 16, 0.0);
        put_f64(&mut list, at + 24, 0.0);
        put_f64(&mut list, at + 32, 16.0);
        put_f64(&mut list, at + 40, 16.0);
        list[blob_at..].copy_from_slice(png);
        list
    }

    fn decode_test_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|error| error.into_inner())
    }

    #[test]
    fn image_decodes_off_the_frame_thread() {
        let _guard = decode_test_lock();
        reset_decode_state();
        reset_image_cache();
        let list = image_list(&TINY_PNG);
        let _ = render(16, 16, &list);
        assert_eq!(cpu_wait_images(2000), 0);
        let out = render(16, 16, &list);
        assert_eq!(rgba(&out, 16, 8, 8), [255, 0, 0, 255]);
    }

    #[test]
    fn svg_image_renders_after_wait() {
        let _guard = decode_test_lock();
        reset_decode_state();
        reset_image_cache();
        let list = image_list(TINY_SVG);
        let _ = render(16, 16, &list);
        assert_eq!(cpu_wait_images(2000), 0);
        let out = render(16, 16, &list);
        assert_eq!(rgba(&out, 16, 8, 8), [10, 20, 30, 255]);
    }

    #[test]
    fn svg_key_tracks_the_display_size() {
        assert_eq!(image_target(TINY_SVG, 16.4, 9.6), (16, 10));
        assert_eq!(image_target(&TINY_PNG, 16.4, 9.6), (0, 0));
        assert_eq!(image_target(TINY_SVG, 0.0, 10.0), (0, 10));
    }

    #[test]
    fn image_pending_reaches_zero_after_wait() {
        let _guard = decode_test_lock();
        reset_decode_state();
        reset_image_cache();
        let list = image_list(&TINY_PNG);
        let _ = render(16, 16, &list);
        assert_eq!(cpu_wait_images(2000), 0);
        assert_eq!(cpu_image_pending(), 0);
    }

    #[test]
    fn invalid_image_fails_once_without_panicking() {
        let _guard = decode_test_lock();
        reset_decode_state();
        reset_image_cache();
        let bad = [0u8; 37];
        let list = image_list(&bad);
        let _ = render(16, 16, &list);
        assert_eq!(cpu_wait_images(2000), 0);
        assert_eq!(cpu_image_pending(), 0);
        let out = render(16, 16, &list);
        assert_eq!(rgba(&out, 16, 8, 8)[3], 0);
        assert_eq!(decode_enqueued(), 1);
    }

    #[test]
    fn same_image_bytes_are_enqueued_once() {
        let _guard = decode_test_lock();
        reset_decode_state();
        reset_image_cache();
        let list = image_list(&TINY_PNG);
        let _ = render(16, 16, &list);
        let _ = render(16, 16, &list);
        assert_eq!(cpu_wait_images(2000), 0);
        let _ = render(16, 16, &list);
        assert_eq!(decode_enqueued(), 1);
    }

    #[test]
    fn ready_map_eviction_stays_bounded() {
        let _guard = decode_test_lock();
        reset_decode_state();
        let image = match scene::decode_png(&TINY_PNG) {
            Some(image) => image,
            None => panic!("tiny png decodes"),
        };
        let mut state = lock_decode_state();
        for i in 0..(DECODE_READY_MAX_ENTRIES + 8) {
            state.store_ready((i as u64, i + 1, 0, 0), image.clone());
        }
        assert!(state.ready.len() <= DECODE_READY_MAX_ENTRIES);
        assert!(state.ready_bytes <= DECODE_READY_MAX_BYTES);
        assert_eq!(state.order.len(), state.ready.len());
    }

    fn gradient_rect_list(kind: f64, radius: f64, stops: &[(u32, f64)]) -> Vec<u8> {
        let blob_at = HEADER + STRIDE;
        let blob_len = stops.len() * 8;
        let mut list = vec![0u8; blob_at + blob_len];
        put_u32(&mut list, 0, 0x4458_4E52);
        put_u32(&mut list, 4, 3);
        put_u32(&mut list, 8, 1);
        put_u32(&mut list, 12, STRIDE as u32);
        put_u32(&mut list, 16, blob_at as u32);
        put_u32(&mut list, 20, blob_len as u32);
        put_u32(&mut list, 24, 64);
        put_u32(&mut list, 28, 64);

        let at = record(&mut list, 0, 25);
        put_u32(&mut list, at + 8, 0);
        put_u32(&mut list, at + 12, stops.len() as u32);
        put_f64(&mut list, at + 16, 0.0);
        put_f64(&mut list, at + 24, 0.0);
        put_f64(&mut list, at + 32, 64.0);
        put_f64(&mut list, at + 40, 64.0);
        put_f64(&mut list, at + 48, kind);
        put_f64(&mut list, at + 56, radius);

        for (i, &(color, offset)) in stops.iter().enumerate() {
            let at = blob_at + i * 8;
            put_u32(&mut list, at, color);
            put_f32(&mut list, at + 4, offset as f32);
        }
        list
    }

    #[test]
    fn paints_three_stop_gradient() {
        let list = gradient_rect_list(
            1.0,
            0.0,
            &[(0xFF00_00FF, 0.0), (0x00FF_00FF, 0.5), (0x0000_FFFF, 1.0)],
        );
        let out = render(64, 64, &list);
        let left = rgba(&out, 64, 1, 32);
        let mid = rgba(&out, 64, 32, 32);
        let right = rgba(&out, 64, 62, 32);
        assert!(
            left[0] > 180 && left[2] < 80,
            "left pixel is not red: {left:?}"
        );
        assert!(
            mid[1] > 180 && mid[0] < 80 && mid[2] < 80,
            "mid pixel is not green: {mid:?}"
        );
        assert!(
            right[2] > 180 && right[0] < 80,
            "right pixel is not blue: {right:?}"
        );
    }

    #[test]
    fn paints_radial_center_and_edge() {
        let list = gradient_rect_list(2.0, 0.0, &[(0xFFFF_FFFF, 0.0), (0x0000_00FF, 1.0)]);
        let out = render(64, 64, &list);
        let center = rgba(&out, 64, 32, 32);
        let edge = rgba(&out, 64, 1, 32);
        assert!(
            center[0] > 200 && center[1] > 200 && center[2] > 200,
            "center pixel is not white: {center:?}"
        );
        assert!(
            edge[0] < 80 && edge[1] < 80 && edge[2] < 80,
            "edge pixel is not black: {edge:?}"
        );
        assert_eq!(center[3], 255);
    }

    #[test]
    fn malformed_gradient_stops_fail_safely() {
        let mut out = vec![0u8; 64 * 64 * 4];
        let mut check = |list: &[u8]| {
            let rc = cpu_render(64, 64, list.as_ptr(), list.len() as i32, out.as_mut_ptr());
            assert!(rc < 0, "malformed gradient rendered with {rc}");
        };

        let too_few = gradient_rect_list(0.0, 0.0, &[(0xFF00_00FF, 0.0)]);
        check(&too_few);

        let mut truncated = gradient_rect_list(0.0, 0.0, &[(0xFF00_00FF, 0.0), (0x00FF_00FF, 1.0)]);
        put_u32(&mut truncated, 20, 8);
        check(&truncated);

        let mut non_finite =
            gradient_rect_list(0.0, 0.0, &[(0xFF00_00FF, 0.0), (0x00FF_00FF, 1.0)]);
        put_u32(&mut non_finite, HEADER + STRIDE + 12, f32::NAN.to_bits());
        check(&non_finite);

        let mut overlong = gradient_rect_list(0.0, 0.0, &[(0xFF00_00FF, 0.0), (0x00FF_00FF, 1.0)]);
        put_u32(&mut overlong, HEADER + 12, u32::MAX);
        check(&overlong);
    }
}
