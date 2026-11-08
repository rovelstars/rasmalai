use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::io::Cursor;
use std::sync::Arc;

use vello::kurbo::{Affine, BezPath, Cap, Join, Point, Rect, RoundedRect, Stroke};
use vello::peniko::{
    BlendMode, Blob, BrushRef, Color, ColorStop, ColorStops, Compose, Extend, Fill, Gradient,
    GradientKind, ImageAlphaType, ImageData, ImageFormat, LinearGradientPosition, Mix,
    RadialGradientPosition,
};

const MAGIC: u32 = 0x4458_4E52;
const STRIDE: usize = 64;
const HEADER: usize = 32;

const FONT_TABLE_MAGIC: u32 = 0x464E_5432;
const FONT_TABLE_HEADER: usize = 12;
const FONT_TABLE_MAX_FONTS: usize = 16;
const FONT_TABLE_MAX_BYTES: usize = 8 * 1024 * 1024;
const FONT_ID_MIN: u32 = 100;
const FONT_ID_MAX: u32 = 115;
const ERR_FONT_TABLE: i32 = -22;
const ERR_FONT_TABLE_SIZE: i32 = -23;

const CACHE_MAX_ENTRIES: usize = 64;
const CACHE_MAX_BYTES: usize = 32 * 1024 * 1024;

struct ImageCacheEntry {
    image: ImageData,
    bytes: usize,
}

struct ImageCache {
    entries: HashMap<(u64, usize), ImageCacheEntry>,
    order: VecDeque<(u64, usize)>,
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

    fn insert(&mut self, key: (u64, usize), image: ImageData, bytes: usize) {
        if bytes > CACHE_MAX_BYTES {
            return;
        }
        while self.entries.len() >= CACHE_MAX_ENTRIES {
            self.evict_oldest();
        }
        while self.bytes + bytes > CACHE_MAX_BYTES {
            self.evict_oldest();
        }
        self.bytes += bytes;
        self.order.push_back(key);
        self.entries.insert(key, ImageCacheEntry { image, bytes });
    }
}

thread_local! {
    static IMAGE_CACHE: RefCell<ImageCache> = RefCell::new(ImageCache::new());
}

const SVG_CACHE_MAX_ENTRIES: usize = 32;
const SVG_CACHE_MAX_BYTES: usize = 32 * 1024 * 1024;
const SVG_MAX_TARGET_SIDE: u32 = 16_384;
const SVG_MAX_TARGET_PIXELS: u64 = 16 * 1024 * 1024;

type SvgKey = (u64, usize, u32, u32);

struct SvgCacheEntry {
    image: ImageData,
    bytes: usize,
}

struct SvgCache {
    entries: HashMap<SvgKey, SvgCacheEntry>,
    order: VecDeque<SvgKey>,
    bytes: usize,
}

impl SvgCache {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            bytes: 0,
        }
    }

    fn evict_oldest(&mut self) {
        if let Some(key) = self.order.pop_front() {
            if let Some(entry) = self.entries.remove(&key) {
                self.bytes -= entry.bytes;
            }
        }
    }

    fn insert(&mut self, key: SvgKey, image: ImageData, bytes: usize) {
        if bytes > SVG_CACHE_MAX_BYTES {
            return;
        }
        while self.entries.len() >= SVG_CACHE_MAX_ENTRIES {
            self.evict_oldest();
        }
        while self.bytes + bytes > SVG_CACHE_MAX_BYTES {
            self.evict_oldest();
        }
        self.bytes += bytes;
        self.order.push_back(key);
        self.entries.insert(key, SvgCacheEntry { image, bytes });
    }
}

thread_local! {
    static SVG_CACHE: RefCell<SvgCache> = RefCell::new(SvgCache::new());
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

struct FontEntry {
    id: u32,
    hash: u64,
    name: String,
    bytes: usize,
}

struct FontRegistry {
    entries: VecDeque<FontEntry>,
    bytes: usize,
}

impl FontRegistry {
    fn new() -> Self {
        Self {
            entries: VecDeque::new(),
            bytes: 0,
        }
    }

    fn family(&self, id: u32) -> Option<String> {
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.name.clone())
    }

    fn name_for(&self, hash: u64, bytes: usize) -> Option<String> {
        self.entries
            .iter()
            .find(|entry| entry.hash == hash && entry.bytes == bytes)
            .map(|entry| entry.name.clone())
    }

    fn insert(&mut self, id: u32, hash: u64, name: String, bytes: usize) {
        if let Some(position) = self.entries.iter().position(|entry| entry.id == id) {
            self.bytes = self.bytes.saturating_sub(self.entries[position].bytes);
            self.entries.remove(position);
        }
        self.bytes = self.bytes.saturating_add(bytes);
        self.entries.push_back(FontEntry {
            id,
            hash,
            name,
            bytes,
        });
    }
}

thread_local! {
    static FONT_REGISTRY: RefCell<FontRegistry> = RefCell::new(FontRegistry::new());
    static FRAME_FAMILIES: RefCell<HashMap<u32, String>> = RefCell::new(HashMap::new());
}

/// One decoded display-list command.
#[derive(Debug)]
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
    RoundShadow {
        color: u32,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        radius: f64,
        std_dev: f64,
    },
    ClipPushRound {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        radius: f64,
    },
    RoundRectGradient {
        top: u32,
        bottom: u32,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        radius: f64,
    },
    GradientRect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        kind: u8,
        radius: f64,
        stops: Vec<(u32, f32)>,
    },
    RoundRectStroke {
        color: u32,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        radius: f64,
        width: f64,
    },
    Text {
        color: u32,
        x: f64,
        y: f64,
        size: f32,
        weight: i32,
        italic: bool,
        tracking: f64,
        family: i32,
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
    PathFill {
        color: u32,
        path: BezPath,
        even_odd: bool,
    },
    PathStroke {
        color: u32,
        path: BezPath,
        width: f64,
        cap: u8,
        join: u8,
    },
    LayerPush {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        alpha: f32,
        blend: u8,
    },
    LayerPop,
    PushMask {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        radius: f64,
    },
    PopMask,
    PushBlur {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        sigma: f64,
    },
    PopBlur,
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    let chunk = bytes.get(at..at + 4)?;
    Some(u32::from_le_bytes(chunk.try_into().ok()?))
}

fn f64_at(bytes: &[u8], at: usize) -> Option<f64> {
    let chunk = bytes.get(at..at + 8)?;
    Some(f64::from_le_bytes(chunk.try_into().ok()?))
}

fn point_at(bytes: &[u8], at: usize) -> Option<Point> {
    Some(Point::new(
        f64_at(bytes, at)?,
        f64_at(bytes, at.checked_add(8)?)?,
    ))
}

fn path_at(blob: &[u8], at: usize, verbs: usize) -> Result<BezPath, i32> {
    let mut path = BezPath::new();
    let mut at = at;
    for _ in 0..verbs {
        let verb = *blob.get(at).ok_or(-21)?;
        at = at.checked_add(1).ok_or(-21)?;
        match verb {
            0 => {
                path.move_to(point_at(blob, at).ok_or(-21)?);
                at = at.checked_add(16).ok_or(-21)?;
            }
            1 => {
                path.line_to(point_at(blob, at).ok_or(-21)?);
                at = at.checked_add(16).ok_or(-21)?;
            }
            2 => {
                let control = point_at(blob, at).ok_or(-21)?;
                let end = point_at(blob, at.checked_add(16).ok_or(-21)?).ok_or(-21)?;
                path.quad_to(control, end);
                at = at.checked_add(32).ok_or(-21)?;
            }
            3 => {
                let c1 = point_at(blob, at).ok_or(-21)?;
                let c2 = point_at(blob, at.checked_add(16).ok_or(-21)?).ok_or(-21)?;
                let end = point_at(blob, at.checked_add(32).ok_or(-21)?).ok_or(-21)?;
                path.curve_to(c1, c2, end);
                at = at.checked_add(48).ok_or(-21)?;
            }
            4 => path.close_path(),
            _ => return Err(-21),
        }
    }
    Ok(path)
}

fn gradient_stops(blob: &[u8], at: usize, count: usize) -> Result<Vec<(u32, f32)>, i32> {
    if count < 2 || count > blob.len() / 8 {
        return Err(-20);
    }
    let len = count.checked_mul(8).ok_or(-20)?;
    let end = at.checked_add(len).ok_or(-20)?;
    let bytes = blob.get(at..end).ok_or(-20)?;
    let mut stops = Vec::with_capacity(count);
    for chunk in bytes.chunks_exact(8) {
        let color = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        let offset = f32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
        if !offset.is_finite() {
            return Err(-20);
        }
        stops.push((color, offset.clamp(0.0, 1.0)));
    }
    stops.sort_by(|a, b| a.1.total_cmp(&b.1));
    Ok(stops)
}

fn gradient_kind(e: f64) -> u8 {
    match if e.is_finite() { e.round() as i64 } else { 0 } {
        1 => 1,
        2 => 2,
        _ => 0,
    }
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

fn normalize_weight(weight: i32) -> i32 {
    if weight <= 0 {
        400
    } else {
        weight.clamp(100, 900)
    }
}

fn normalize_family(family: i32) -> i32 {
    if (0..=11).contains(&family) || family >= 100 {
        family
    } else {
        0
    }
}

/// Split the free text lanes into `(weight, italic, tracking, family)`.
///
/// `e` packs `weight + (italic ? 10000 : 0) + family_id * 1_000_000`. Records
/// written before families existed leave the family field clear, so `0` reads
/// as sans-serif at the default weight and `10700` as bold italic sans-serif.
fn text_style(e: f64, f: f64) -> (i32, bool, f64, i32) {
    let packed = if e.is_finite() { e.round() as i32 } else { 0 };
    let family = normalize_family(packed / 1_000_000);
    let styles = packed % 1_000_000;
    let italic = styles >= 10000;
    let raw = if italic { styles - 10000 } else { styles };
    let tracking = if f.is_finite() { f } else { 0.0 };
    (normalize_weight(raw), italic, tracking, family)
}

fn family_of(id: i32) -> parley::GenericFamily {
    match id {
        1 => parley::GenericFamily::Serif,
        2 => parley::GenericFamily::Monospace,
        3 => parley::GenericFamily::Cursive,
        4 => parley::GenericFamily::Fantasy,
        5 => parley::GenericFamily::SystemUi,
        6 => parley::GenericFamily::UiSerif,
        7 => parley::GenericFamily::UiSansSerif,
        8 => parley::GenericFamily::UiMonospace,
        9 => parley::GenericFamily::UiRounded,
        10 => parley::GenericFamily::Emoji,
        11 => parley::GenericFamily::Math,
        _ => parley::GenericFamily::SansSerif,
    }
}

struct FontTableEntry<'a> {
    id: u32,
    name: &'a str,
}

fn parse_font_table(blob: &[u8]) -> Result<Vec<FontTableEntry<'_>>, i32> {
    if u32_at(blob, 0) != Some(FONT_TABLE_MAGIC) {
        return Ok(Vec::new());
    }
    let table_len = u32_at(blob, 4).ok_or(ERR_FONT_TABLE)? as usize;
    if table_len < FONT_TABLE_HEADER || table_len > blob.len() {
        return Err(ERR_FONT_TABLE);
    }
    let count = u32_at(blob, 8).ok_or(ERR_FONT_TABLE)? as usize;
    if count > FONT_TABLE_MAX_FONTS {
        return Err(ERR_FONT_TABLE_SIZE);
    }
    let table = blob.get(..table_len).ok_or(ERR_FONT_TABLE)?;
    let mut at = FONT_TABLE_HEADER;
    let mut entries: Vec<FontTableEntry<'_>> = Vec::with_capacity(count);
    for _ in 0..count {
        let id = u32_at(table, at).ok_or(ERR_FONT_TABLE)?;
        at = at.checked_add(4).ok_or(ERR_FONT_TABLE)?;
        let name_len = u32_at(table, at).ok_or(ERR_FONT_TABLE)? as usize;
        at = at.checked_add(4).ok_or(ERR_FONT_TABLE)?;
        if name_len == 0 {
            return Err(ERR_FONT_TABLE);
        }
        let name_end = at.checked_add(name_len).ok_or(ERR_FONT_TABLE)?;
        let name = std::str::from_utf8(table.get(at..name_end).ok_or(ERR_FONT_TABLE)?)
            .map_err(|_| ERR_FONT_TABLE)?;
        if !(FONT_ID_MIN..=FONT_ID_MAX).contains(&id) {
            return Err(ERR_FONT_TABLE);
        }
        if entries.iter().any(|entry| entry.id == id) {
            return Err(ERR_FONT_TABLE);
        }
        entries.push(FontTableEntry { id, name });
        at = name_end;
    }
    Ok(entries)
}

/// Register a named font with the thread-local shaping context.
///
/// Returns 0 on success, -1 for invalid arguments or unusable font data, and
/// -2 when the 16-face or 8 MiB cap would be exceeded.
pub fn register_font(id: u32, name: &str, data: &[u8]) -> i32 {
    if !(FONT_ID_MIN..=FONT_ID_MAX).contains(&id) || name.is_empty() || data.is_empty() {
        return -1;
    }
    let hash = fnv1a(data);
    let existing = FONT_REGISTRY.with(|registry| {
        registry
            .borrow()
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| (entry.hash, entry.bytes))
    });
    if existing.is_some_and(|(registered_hash, _)| registered_hash == hash) {
        return 0;
    }
    let fits = FONT_REGISTRY.with(|registry| {
        let registry = registry.borrow();
        let faces = registry
            .entries
            .iter()
            .filter(|entry| entry.id != id)
            .count();
        let bytes = registry
            .bytes
            .saturating_sub(existing.map_or(0, |(_, bytes)| bytes));
        faces < FONT_TABLE_MAX_FONTS
            && bytes.saturating_add(data.len()) <= FONT_TABLE_MAX_BYTES
    });
    if !fits {
        return -2;
    }
    let blob = parley::fontique::Blob::new(Arc::new(data.to_vec()));
    let info = parley::fontique::FontInfoOverride {
        family_name: Some(name),
        width: None,
        style: None,
        weight: None,
        axes: None,
    };
    let registered = SHAPER.with(|slot| {
        let mut slot = slot.borrow_mut();
        let (font_cx, _) = slot.get_or_insert_with(|| {
            (
                parley::FontContext::new(),
                parley::LayoutContext::<()>::new(),
            )
        });
        !font_cx
            .collection
            .register_fonts(blob, Some(info))
            .is_empty()
    });
    if !registered {
        return -1;
    }
    FONT_REGISTRY.with(|registry| {
        registry
            .borrow_mut()
            .insert(id, hash, name.to_string(), data.len());
    });
    CACHE.with(|cache| cache.borrow_mut().clear());
    0
}

fn registered_family(id: i32) -> Option<String> {
    FRAME_FAMILIES
        .with(|map| map.borrow().get(&(id as u32)).cloned())
        .or_else(|| FONT_REGISTRY.with(|registry| registry.borrow().family(id as u32)))
}

fn family_name_for_font(font: &parley::FontData) -> Option<String> {
    let data = font.data.data();
    let hash = fnv1a(data);
    FONT_REGISTRY.with(|registry| registry.borrow().name_for(hash, data.len()))
}

/// Decode display-list v2 and v3 into commands.
pub fn parse(bytes: &[u8]) -> Result<Vec<Command>, i32> {
    if bytes.len() < HEADER {
        return Err(-1);
    }
    if u32_at(bytes, 0) != Some(MAGIC) {
        return Err(-10);
    }
    match u32_at(bytes, 4) {
        Some(2) | Some(3) => {}
        _ => return Err(-11),
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

    let font_entries = parse_font_table(blob)?;

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
                let (weight, italic, tracking, family) = text_style(e, f);
                out.push(Command::Text {
                    color,
                    x,
                    y,
                    size: w as f32,
                    weight,
                    italic,
                    tracking,
                    family,
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
            10 => out.push(Command::RoundRectGradient {
                top: color,
                bottom: a as u32,
                x,
                y,
                w,
                h,
                radius: e,
            }),
            25 => out.push(Command::GradientRect {
                x,
                y,
                w,
                h,
                kind: gradient_kind(e),
                radius: if f.is_finite() && f > 0.0 { f } else { 0.0 },
                stops: gradient_stops(blob, a, b)?,
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
            11 => out.push(Command::PathFill {
                color,
                path: path_at(blob, a, b)?,
                even_odd: e != 0.0,
            }),
            12 => {
                let bits = f as u32 as u8;
                out.push(Command::PathStroke {
                    color,
                    path: path_at(blob, a, b)?,
                    width: e,
                    cap: bits & 3,
                    join: (bits >> 2) & 3,
                });
            }
            13 => out.push(Command::LayerPush {
                x,
                y,
                w,
                h,
                alpha: e as f32,
                blend: f as u8,
            }),
            14 => out.push(Command::LayerPop),
            15 => out.push(Command::RoundRectStroke {
                color,
                x,
                y,
                w,
                h,
                radius: e,
                width: f,
            }),
            17 => out.push(Command::RoundShadow {
                color,
                x,
                y,
                w,
                h,
                radius: e,
                std_dev: f,
            }),
            18 => out.push(Command::ClipPushRound {
                x,
                y,
                w,
                h,
                radius: e,
            }),
            19 => out.push(Command::PushMask {
                x,
                y,
                w,
                h,
                radius: e,
            }),
            20 => out.push(Command::PopMask),
            21 => out.push(Command::PushBlur {
                x,
                y,
                w,
                h,
                sigma: e,
            }),
            22 => out.push(Command::PopBlur),
            _ => {}
        }
    }
    let families: HashMap<u32, String> = font_entries
        .into_iter()
        .map(|entry| (entry.id, entry.name.to_string()))
        .collect();
    FRAME_FAMILIES.with(|map| *map.borrow_mut() = families);
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

fn gradient_brush(
    kind: u8,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    radius: f64,
    stops: &[(u32, f32)],
) -> Gradient {
    let ramp: Vec<ColorStop> = stops
        .iter()
        .map(|&(color, offset)| ColorStop {
            offset,
            color: color_of(color).into(),
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
    Gradient {
        kind,
        extend: Extend::Pad,
        stops: ColorStops::from(&ramp[..]),
        ..Default::default()
    }
}

#[derive(Clone, Copy)]
enum ImageFormatKind {
    Png,
    Jpeg,
    WebP,
}

fn sniff_format(bytes: &[u8]) -> Option<ImageFormatKind> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        Some(ImageFormatKind::Png)
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        Some(ImageFormatKind::Jpeg)
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(&b"WEBP"[..]) {
        Some(ImageFormatKind::WebP)
    } else {
        None
    }
}

fn image_rgba8(pixels: Vec<u8>, width: u32, height: u32) -> ImageData {
    ImageData {
        data: Blob::new(Arc::new(pixels)),
        format: ImageFormat::Rgba8,
        alpha_type: vello::peniko::ImageAlphaType::Alpha,
        width,
        height,
    }
}

fn into_rgba8(pixels: Vec<u8>, width: u32, height: u32) -> Option<ImageData> {
    let count = (width as usize).checked_mul(height as usize)?;
    if count == 0 {
        return None;
    }
    let rgba_len = count.checked_mul(4)?;
    if pixels.len() == rgba_len {
        return Some(image_rgba8(pixels, width, height));
    }
    let rgb_len = count.checked_mul(3)?;
    let mut out = Vec::with_capacity(rgba_len);
    if pixels.len() == rgb_len {
        for pixel in pixels.chunks_exact(3) {
            out.extend_from_slice(pixel);
            out.push(255);
        }
    } else if pixels.len() == count {
        for value in pixels {
            out.extend_from_slice(&[value, value, value, 255]);
        }
    } else {
        return None;
    }
    Some(image_rgba8(out, width, height))
}

/// Decode a PNG, JPEG, WebP, or SVG into RGBA8 image data.
///
/// Raster images decode at their intrinsic size. SVG documents decode at their
/// intrinsic size as well; use [`decode_image_at`] to rasterize at another size.
/// Decoded images are cached per thread, keyed by content hash and length.
pub fn decode_png(bytes: &[u8]) -> Option<ImageData> {
    decode_image_at(bytes, 0, 0)
}

/// Decode an image, rasterizing SVG at `target_w` by `target_h`.
///
/// Both target dimensions must be non-zero for SVG rasterization. Otherwise the
/// SVG intrinsic size is used, rounded up with a minimum of one pixel per side.
/// Raster formats always decode at their intrinsic size and ignore the target.
pub fn decode_image_at(bytes: &[u8], target_w: u32, target_h: u32) -> Option<ImageData> {
    if is_raw_frame(bytes) {
        decode_raw_frame(bytes)
    } else if is_svg(bytes) {
        decode_svg_at(bytes, target_w, target_h)
    } else {
        decode_raster_cached(bytes)
    }
}

/// Raw RGBA frames from the media wrapper: 24-byte header plus pixels.
pub fn is_raw_frame(bytes: &[u8]) -> bool {
    u32_at(bytes, 0) == Some(0x5258_4E52)
}

fn decode_raw_frame(bytes: &[u8]) -> Option<ImageData> {
    if bytes.len() < 24 {
        return None;
    }
    let width = u32_at(bytes, 4)?;
    let height = u32_at(bytes, 8)?;
    let stride = u32_at(bytes, 12)? as usize;
    let format = u32_at(bytes, 16)?;
    let len = u32_at(bytes, 20)? as usize;
    if width == 0 || height == 0 || format != 0 || stride != width as usize * 4 {
        return None;
    }
    let need = stride.checked_mul(height as usize)?;
    if len != need || bytes.len() < 24 + len {
        return None;
    }
    let pixels = bytes.get(24..24 + len)?.to_vec();
    Some(ImageData {
        data: Blob::new(Arc::new(pixels)),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::Alpha,
        width,
        height,
    })
}

/// Return true when `bytes` starts with an SVG document after a BOM and whitespace.
pub fn is_svg(bytes: &[u8]) -> bool {
    let start = if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        3
    } else {
        0
    };
    let rest = &bytes[start..];
    let mut index = 0;
    while index < rest.len() && rest[index].is_ascii_whitespace() {
        index += 1;
    }
    rest[index..].starts_with(b"<svg")
}

fn decode_raster_cached(bytes: &[u8]) -> Option<ImageData> {
    let key = (fnv1a(bytes), bytes.len());
    let cached = IMAGE_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        match cache.entries.get(&key) {
            Some(entry) => {
                let image = entry.image.clone();
                cache.hits += 1;
                Some(image)
            }
            None => {
                cache.misses += 1;
                None
            }
        }
    });
    if cached.is_some() {
        return cached;
    }
    let image = decode_uncached(bytes)?;
    let bytes = image.data.data().len();
    IMAGE_CACHE.with(|cache| cache.borrow_mut().insert(key, image.clone(), bytes));
    Some(image)
}

fn decode_svg_at(bytes: &[u8], target_w: u32, target_h: u32) -> Option<ImageData> {
    let key = (fnv1a(bytes), bytes.len(), target_w, target_h);
    let cached = SVG_CACHE.with(|cache| {
        cache
            .borrow()
            .entries
            .get(&key)
            .map(|entry| entry.image.clone())
    });
    if let Some(image) = cached {
        return Some(image);
    }
    let image = rasterize_svg(bytes, target_w, target_h)?;
    let size = image.data.data().len();
    SVG_CACHE.with(|cache| cache.borrow_mut().insert(key, image.clone(), size));
    Some(image)
}

fn rasterize_svg(bytes: &[u8], target_w: u32, target_h: u32) -> Option<ImageData> {
    let tree = resvg::usvg::Tree::from_data(bytes, &resvg::usvg::Options::default()).ok()?;
    let natural = tree.size();
    let natural_w = natural.width();
    let natural_h = natural.height();
    if !natural_w.is_finite() || !natural_h.is_finite() || natural_w <= 0.0 || natural_h <= 0.0 {
        return None;
    }
    let (width, height) = if target_w > 0 && target_h > 0 {
        (target_w, target_h)
    } else {
        (natural_w.ceil().max(1.0) as u32, natural_h.ceil().max(1.0) as u32)
    };
    if width == 0 || height == 0 {
        return None;
    }
    if width > SVG_MAX_TARGET_SIDE || height > SVG_MAX_TARGET_SIDE {
        return None;
    }
    if (width as u64).checked_mul(height as u64)? > SVG_MAX_TARGET_PIXELS {
        return None;
    }
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)?;
    let transform = resvg::tiny_skia::Transform::from_scale(
        width as f32 / natural_w,
        height as f32 / natural_h,
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let pixels = pixmap.take();
    let count = (width as usize).checked_mul(height as usize)?;
    if pixels.len() != count.checked_mul(4)? {
        return None;
    }
    Some(ImageData {
        data: Blob::new(Arc::new(pixels)),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::AlphaPremultiplied,
        width,
        height,
    })
}

/// Return `(hits, misses, entries, bytes)` for this thread's decoded-image cache.
pub fn png_cache_stats() -> (u64, u64, usize, usize) {
    IMAGE_CACHE.with(|cache| {
        let cache = cache.borrow();
        (cache.hits, cache.misses, cache.entries.len(), cache.bytes)
    })
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

#[cfg(test)]
fn reset_svg_cache() {
    SVG_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.entries.clear();
        cache.order.clear();
        cache.bytes = 0;
    });
}

fn decode_uncached(bytes: &[u8]) -> Option<ImageData> {
    match sniff_format(bytes)? {
        ImageFormatKind::Png => decode_png_uncached(bytes),
        ImageFormatKind::Jpeg => decode_jpeg(bytes),
        ImageFormatKind::WebP => decode_webp(bytes),
    }
}

fn decode_png_uncached(bytes: &[u8]) -> Option<ImageData> {
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
    Some(image_rgba8(buf, info.width, info.height))
}

fn decode_jpeg(bytes: &[u8]) -> Option<ImageData> {
    use zune_jpeg::zune_core::bytestream::ZCursor;
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;

    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGBA);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), options);
    let pixels = decoder.decode().ok()?;
    let info = decoder.info()?;
    into_rgba8(pixels, info.width as u32, info.height as u32)
}

fn decode_webp(bytes: &[u8]) -> Option<ImageData> {
    let mut decoder = image_webp::WebPDecoder::new(Cursor::new(bytes)).ok()?;
    let (width, height) = decoder.dimensions();
    let mut pixels = vec![0; decoder.output_buffer_size()?];
    decoder.read_image(&mut pixels).ok()?;
    into_rgba8(pixels, width, height)
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
    family_name: Option<String>,
}

/// Borrowed view of a shaped run's font and registered family name.
pub struct FontRef<'a> {
    data: &'a parley::FontData,
    name: Option<&'a str>,
}

impl<'a> FontRef<'a> {
    pub fn data(&self) -> &'a parley::FontData {
        self.data
    }

    pub fn name(&self) -> Option<&'a str> {
        self.name
    }
}

impl ShapedRun {
    pub fn font(&self) -> FontRef<'_> {
        FontRef {
            data: &self.font,
            name: self.family_name.as_deref(),
        }
    }
}

thread_local! {
    static SHAPER: RefCell<Option<(parley::FontContext, parley::LayoutContext<()>)>> =
        const { RefCell::new(None) };
    static CACHE: RefCell<
        HashMap<(u32, i32, bool, u64, i32, Option<String>, String), Vec<ShapedRun>>,
    > = RefCell::new(HashMap::new());
}

/// Shape text once and reuse the result. Fonts are discovered once per thread.
pub fn shape(text: &str, size: f32) -> Vec<ShapedRun> {
    shape_styled(text, size, 400, false, 0.0, 0)
}

/// Shape text with a weight, slant, and extra per-character advance. The
/// result is cached per thread by text and every style field.
pub fn shape_styled(
    text: &str,
    size: f32,
    weight: i32,
    italic: bool,
    tracking: f64,
    family: i32,
) -> Vec<ShapedRun> {
    let weight = normalize_weight(weight);
    let tracking = if tracking.is_finite() { tracking } else { 0.0 };
    let family = normalize_family(family);
    let family_name = if family >= 100 {
        registered_family(family)
    } else {
        None
    };
    let key = (
        size.to_bits(),
        weight,
        italic,
        tracking.to_bits(),
        family,
        family_name.clone(),
        text.to_string(),
    );
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
        match &family_name {
            Some(name) => builder.push_default(parley::StyleProperty::FontFamily(
                parley::FontFamily::named(name),
            )),
            None => {
                builder.push_default(parley::StyleProperty::FontFamily(family_of(family).into()))
            }
        }
        if weight != 400 {
            builder.push_default(parley::StyleProperty::FontWeight(parley::FontWeight::new(
                weight as f32,
            )));
        }
        if italic {
            builder.push_default(parley::StyleProperty::FontStyle(parley::FontStyle::Italic));
        }
        if tracking != 0.0 {
            builder.push_default(parley::StyleProperty::LetterSpacing(tracking as f32));
        }
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
                    let run_family =
                        family_name_for_font(shaped.font()).or_else(|| family_name.clone());
                    runs.push(ShapedRun {
                        font: shaped.font().clone(),
                        size: shaped.font_size(),
                        glyphs,
                        family_name: run_family,
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
    to_gpu_with(commands, Affine::IDENTITY)
}

/// Turn decoded commands into a scene shifted by `dx`, `dy`.
pub fn to_gpu_at(commands: &[Command], dx: f64, dy: f64) -> vello::Scene {
    to_gpu_with(commands, Affine::translate((dx, dy)))
}

/// Turn decoded commands into a scene shifted by `start`.
pub fn to_gpu_transformed(commands: &[Command], start: Affine) -> vello::Scene {
    to_gpu_with(commands, start)
}

/// One blur group found in a display list.
pub struct BlurSpec {
    pub start: usize,
    pub end: usize,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub sigma: f64,
    pub transform: Affine,
}

/// Collect the top-level blur groups of `commands`.
///
/// `start` and `end` are the body command indices, exclusive of the push and
/// pop records. Nested blur groups are left inside the body for the caller to
/// collect again against `start`.
pub fn collect_blurs(commands: &[Command], start: Affine) -> Vec<BlurSpec> {
    let mut out = Vec::new();
    let mut transform = start;
    let mut transforms: Vec<Affine> = Vec::new();
    let mut open: Option<(usize, Affine)> = None;
    let mut depth = 0usize;
    for (index, command) in commands.iter().enumerate() {
        match command {
            Command::TransformPush(m) => {
                transforms.push(transform);
                transform = transform * Affine::new(*m);
            }
            Command::TransformPop => {
                transform = transforms.pop().unwrap_or(Affine::IDENTITY);
            }
            Command::PushBlur { .. } => {
                if depth == 0 {
                    open = Some((index, transform));
                }
                depth += 1;
            }
            Command::PopBlur => {
                if depth == 0 {
                    continue;
                }
                depth -= 1;
                if depth == 0 {
                    if let Some((push, at)) = open.take() {
                        if let Command::PushBlur { x, y, w, h, sigma } = commands[push] {
                            out.push(BlurSpec {
                                start: push + 1,
                                end: index,
                                x,
                                y,
                                w,
                                h,
                                sigma,
                                transform: at,
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Turn decoded commands into a scene using pre-blurred group images.
///
/// `images[i]` paints the i-th `PushBlur` group. A missing image leaves the
/// group painted by the normal unblurred path.
pub fn to_gpu_images(
    commands: &[Command],
    start: Affine,
    images: &[ImageData],
) -> vello::Scene {
    build_scene(commands, start, Some(images))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LayerKind {
    Clip,
    Mask,
    Blur,
}

fn close_layers(scene: &mut vello::Scene, layers: &mut Vec<LayerKind>) {
    while let Some(kind) = layers.pop() {
        if kind != LayerKind::Blur {
            scene.pop_layer();
        }
    }
}

fn to_gpu_with(commands: &[Command], start: Affine) -> vello::Scene {
    build_scene(commands, start, None)
}

fn blur_body_end(commands: &[Command], push: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (index, command) in commands.iter().enumerate().skip(push + 1) {
        match command {
            Command::PushBlur { .. } => depth += 1,
            Command::PopBlur => {
                if depth == 0 {
                    return Some(index);
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

fn build_scene(commands: &[Command], start: Affine, images: Option<&[ImageData]>) -> vello::Scene {
    let mut scene = vello::Scene::new();
    let mut transform = start;
    let mut stack: Vec<Affine> = Vec::new();
    let mut layers: Vec<LayerKind> = Vec::new();
    let mut blur_index = 0usize;
    let mut index = 0usize;
    while index < commands.len() {
        let command = &commands[index];
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
            Command::RoundShadow {
                color,
                x,
                y,
                w,
                h,
                radius,
                std_dev,
            } => {
                scene.draw_blurred_rounded_rect(
                    transform,
                    Rect::new(*x, *y, x + w, y + h),
                    color_of(*color),
                    *radius,
                    *std_dev,
                );
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
                scene.stroke(
                    &Stroke::new(*width),
                    transform,
                    color_of(*color),
                    None,
                    &RoundedRect::new(*x, *y, x + w, y + h, *radius),
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
                let stops = [
                    ColorStop {
                        offset: 0.0,
                        color: color_of(*top).into(),
                    },
                    ColorStop {
                        offset: 1.0,
                        color: color_of(*bottom).into(),
                    },
                ];
                let gradient = Gradient {
                    kind: GradientKind::Linear(LinearGradientPosition {
                        start: Point::new(*x, *y),
                        end: Point::new(*x, *y + *h),
                    }),
                    extend: Extend::Pad,
                    stops: ColorStops::from(&stops[..]),
                    ..Default::default()
                };
                scene.fill(
                    Fill::NonZero,
                    transform,
                    BrushRef::Gradient(&gradient),
                    None,
                    &RoundedRect::new(*x, *y, x + w, y + h, *radius),
                );
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
                let gradient = gradient_brush(*kind, *x, *y, *w, *h, *radius, stops);
                scene.fill(
                    Fill::NonZero,
                    transform,
                    BrushRef::Gradient(&gradient),
                    None,
                    &Rect::new(*x, *y, x + w, y + h),
                );
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
            } => {
                for run in shape_styled(text, *size, *weight, *italic, *tracking, *family) {
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
                layers.push(LayerKind::Clip);
            }
            Command::ClipPushRound { x, y, w, h, radius } => {
                scene.push_clip_layer(
                    Fill::NonZero,
                    transform,
                    &RoundedRect::new(*x, *y, x + w, y + h, *radius),
                );
                layers.push(LayerKind::Clip);
            }
            Command::Polygon { color, points } => {
                if let Some(path) = poly_path(points) {
                    scene.fill(Fill::NonZero, transform, color_of(*color), None, &path);
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
                scene.fill(rule, transform, color_of(*color), None, path);
            }
            Command::PathStroke {
                color,
                path,
                width,
                cap,
                join,
            } => {
                let stroke = Stroke::new(*width)
                    .with_caps(cap_of(*cap))
                    .with_join(join_of(*join));
                scene.stroke(&stroke, transform, color_of(*color), None, path);
            }
            Command::LayerPush {
                x,
                y,
                w,
                h,
                alpha,
                blend,
            } => {
                scene.push_layer(
                    Fill::NonZero,
                    blend_of(*blend),
                    *alpha,
                    transform,
                    &Rect::new(*x, *y, x + w, y + h),
                );
                layers.push(LayerKind::Clip);
            }
            Command::ClipPop | Command::LayerPop => {
                if layers.last() == Some(&LayerKind::Clip) {
                    layers.pop();
                    scene.pop_layer();
                }
            }
            Command::PushMask { x, y, w, h, radius } => {
                if *radius > 0.0 {
                    scene.push_clip_layer(
                        Fill::NonZero,
                        transform,
                        &RoundedRect::new(*x, *y, x + w, y + h, *radius),
                    );
                } else {
                    scene.push_clip_layer(
                        Fill::NonZero,
                        transform,
                        &Rect::new(*x, *y, x + w, y + h),
                    );
                }
                layers.push(LayerKind::Mask);
            }
            Command::PopMask => {
                if layers.last() == Some(&LayerKind::Mask) {
                    layers.pop();
                    scene.pop_layer();
                }
            }
            Command::PushBlur { .. } => {
                let available = images
                    .and_then(|images| images.get(blur_index))
                    .and_then(|image| blur_body_end(commands, index).map(|end| (image, end)));
                blur_index += 1;
                if let Some((image, end)) = available {
                    scene.draw_image(image, Affine::IDENTITY);
                    index = end;
                } else {
                    layers.push(LayerKind::Blur);
                }
            }
            Command::PopBlur => {
                if layers.last() == Some(&LayerKind::Blur) {
                    layers.pop();
                }
            }
            Command::TransformPush(m) => {
                stack.push(transform);
                transform = transform * Affine::new(*m);
            }
            Command::TransformPop => {
                transform = stack.pop().unwrap_or(Affine::IDENTITY);
            }
        }
        index += 1;
    }
    close_layers(&mut scene, &mut layers);
    scene
}

/// Decode display-list v2 and v3 into a vello scene.
pub fn from_list(bytes: &[u8]) -> Result<vello::Scene, i32> {
    Ok(to_gpu(&parse(bytes)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient_list() -> Vec<u8> {
        let mut out = vec![0u8; HEADER + STRIDE];
        out[0..4].copy_from_slice(&MAGIC.to_le_bytes());
        out[4..8].copy_from_slice(&2u32.to_le_bytes());
        out[8..12].copy_from_slice(&1u32.to_le_bytes());
        out[12..16].copy_from_slice(&(STRIDE as u32).to_le_bytes());
        out[16..20].copy_from_slice(&((HEADER + STRIDE) as u32).to_le_bytes());
        out[24..28].copy_from_slice(&100u32.to_le_bytes());
        out[28..32].copy_from_slice(&100u32.to_le_bytes());
        let at = HEADER;
        out[at..at + 4].copy_from_slice(&10u32.to_le_bytes());
        out[at + 4..at + 8].copy_from_slice(&0x303030ffu32.to_le_bytes());
        out[at + 8..at + 12].copy_from_slice(&0x101010ffu32.to_le_bytes());
        out[at + 16..at + 24].copy_from_slice(&8f64.to_le_bytes());
        out[at + 24..at + 32].copy_from_slice(&8f64.to_le_bytes());
        out[at + 32..at + 40].copy_from_slice(&48f64.to_le_bytes());
        out[at + 40..at + 48].copy_from_slice(&48f64.to_le_bytes());
        out[at + 48..at + 56].copy_from_slice(&24f64.to_le_bytes());
        out
    }

    #[test]
    fn parses_round_rect_gradient() {
        let commands = parse(&gradient_list()).expect("parse gradient record");
        match commands.as_slice() {
            [Command::RoundRectGradient {
                top,
                bottom,
                x,
                y,
                w,
                h,
                radius,
            }] => {
                assert_eq!(*top, 0x303030ff);
                assert_eq!(*bottom, 0x101010ff);
                assert_eq!((*x, *y, *w, *h, *radius), (8.0, 8.0, 48.0, 48.0, 24.0));
            }
            other => panic!("unexpected commands: {other:?}"),
        }
        let _ = to_gpu(&commands);
    }

    fn put_u32(out: &mut [u8], at: usize, value: u32) {
        out[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put_f64(out: &mut [u8], at: usize, value: f64) {
        out[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn put_f32(out: &mut [u8], at: usize, value: f32) {
        out[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn push_verb(blob: &mut Vec<u8>, verb: u8, args: &[f64]) {
        blob.push(verb);
        for value in args {
            blob.extend_from_slice(&value.to_le_bytes());
        }
    }

    fn text_list(e: f64, f: f64) -> Vec<u8> {
        let text = b"Styled";
        let blob_at = HEADER + STRIDE;
        let mut out = vec![0u8; blob_at + text.len()];
        put_u32(&mut out, 0, MAGIC);
        put_u32(&mut out, 4, 3);
        put_u32(&mut out, 8, 1);
        put_u32(&mut out, 12, STRIDE as u32);
        put_u32(&mut out, 16, blob_at as u32);
        put_u32(&mut out, 20, text.len() as u32);
        out[blob_at..].copy_from_slice(text);

        let at = HEADER;
        put_u32(&mut out, at, 2);
        put_u32(&mut out, at + 4, 0xFF112233);
        put_u32(&mut out, at + 8, 0);
        put_u32(&mut out, at + 12, text.len() as u32);
        put_f64(&mut out, at + 16, 4.0);
        put_f64(&mut out, at + 24, 8.0);
        put_f64(&mut out, at + 32, 18.0);
        put_f64(&mut out, at + 48, e);
        put_f64(&mut out, at + 56, f);
        out
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
        let table_len = FONT_TABLE_HEADER + body.len();
        let mut out = Vec::with_capacity(table_len);
        out.extend_from_slice(&FONT_TABLE_MAGIC.to_le_bytes());
        out.extend_from_slice(&(table_len as u32).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    fn text_list_with_blob(prefix: &[u8], e: f64, f: f64) -> Vec<u8> {
        let text = b"Inter Probe";
        let blob_at = HEADER + STRIDE;
        let mut out = vec![0u8; blob_at + prefix.len() + text.len()];
        put_u32(&mut out, 0, MAGIC);
        put_u32(&mut out, 4, 3);
        put_u32(&mut out, 8, 1);
        put_u32(&mut out, 12, STRIDE as u32);
        put_u32(&mut out, 16, blob_at as u32);
        put_u32(&mut out, 20, (prefix.len() + text.len()) as u32);
        out[blob_at..blob_at + prefix.len()].copy_from_slice(prefix);
        out[blob_at + prefix.len()..].copy_from_slice(text);

        let at = HEADER;
        put_u32(&mut out, at, 2);
        put_u32(&mut out, at + 4, 0xFF112233);
        put_u32(&mut out, at + 8, prefix.len() as u32);
        put_u32(&mut out, at + 12, text.len() as u32);
        put_f64(&mut out, at + 16, 4.0);
        put_f64(&mut out, at + 24, 8.0);
        put_f64(&mut out, at + 32, 18.0);
        put_f64(&mut out, at + 48, e);
        put_f64(&mut out, at + 56, f);
        out
    }

    #[test]
    fn v2_font_table_parses_ids_and_names() {
        let table = font_table(&[(100, "Inter SemiBold"), (101, "Second Family")]);
        let entries = parse_font_table(&table).expect("parse v2 font table");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, 100);
        assert_eq!(entries[0].name, "Inter SemiBold");
        assert_eq!(entries[1].id, 101);
        assert_eq!(entries[1].name, "Second Family");

        let commands = parse(&text_list_with_blob(&table, 100_000_400.0, 0.0))
            .expect("parse list with v2 table");
        match commands.as_slice() {
            [Command::Text { family, weight, .. }] => {
                assert_eq!(*family, 100);
                assert_eq!(*weight, 400);
            }
            other => panic!("unexpected commands: {other:?}"),
        }

        let mut old = table.clone();
        put_u32(&mut old, 0, 0x464E_5431);
        assert!(
            parse_font_table(&old)
                .expect("old font table magic is ignored")
                .is_empty()
        );
    }

    #[test]
    fn registered_named_family_shapes_with_inter() {
        assert_eq!(register_font(100, "Inter SemiBold", INTER), 0);
        let table = font_table(&[(100, "Inter SemiBold")]);
        let commands =
            parse(&text_list_with_blob(&table, 100_000_400.0, 0.0)).expect("parse font table list");
        match commands.as_slice() {
            [Command::Text { family, weight, .. }] => {
                assert_eq!(*family, 100);
                assert_eq!(*weight, 400);
            }
            other => panic!("unexpected commands: {other:?}"),
        }

        let runs = shape_styled("Inter Probe", 18.0, 400, false, 0.0, 100);
        assert!(!runs.is_empty(), "named family shaping produced no runs");
        assert!(
            runs.iter()
                .any(|run| run.font().name().is_some_and(|name| name.contains("Inter"))),
            "no run reports the Inter family"
        );
        let scene = to_gpu(&commands);
        assert_eq!(scene.encoding().n_open_clips, 0);
    }

    #[test]
    fn register_font_rejects_invalid_arguments_and_repeats() {
        assert_eq!(register_font(99, "Inter SemiBold", INTER), -1);
        assert_eq!(register_font(116, "Inter SemiBold", INTER), -1);
        assert_eq!(register_font(100, "", INTER), -1);
        assert_eq!(register_font(100, "Inter SemiBold", &[]), -1);
        assert_eq!(register_font(100, "Inter SemiBold", INTER), 0);
        assert_eq!(register_font(100, "Inter SemiBold", INTER), 0);
        assert_eq!(FONT_REGISTRY.with(|registry| registry.borrow().entries.len()), 1);
        let oversized = vec![0u8; FONT_TABLE_MAX_BYTES + 1];
        assert_eq!(register_font(101, "Oversized", &oversized), -2);
        assert_eq!(FONT_REGISTRY.with(|registry| registry.borrow().entries.len()), 1);
    }

    #[test]
    fn malformed_font_tables_fail_safely() {
        let mut table = font_table(&[(100, "Inter SemiBold")]);
        let bad_len = table.len() + 64;
        put_u32(&mut table, 4, bad_len as u32);
        assert_eq!(
            parse(&text_list_with_blob(&table, 100_000_400.0, 0.0)).err(),
            Some(ERR_FONT_TABLE)
        );

        let mut table = font_table(&[(100, "Inter SemiBold")]);
        put_u32(&mut table, 8, (FONT_TABLE_MAX_FONTS + 1) as u32);
        assert_eq!(
            parse(&text_list_with_blob(&table, 100_000_400.0, 0.0)).err(),
            Some(ERR_FONT_TABLE_SIZE)
        );

        let mut table = font_table(&[(100, "Inter SemiBold")]);
        let name_len_at = FONT_TABLE_HEADER + 4;
        put_u32(&mut table, name_len_at, 4096);
        assert_eq!(
            parse(&text_list_with_blob(&table, 100_000_400.0, 0.0)).err(),
            Some(ERR_FONT_TABLE)
        );

        let truncated = &font_table(&[(100, "Inter SemiBold")])[..FONT_TABLE_HEADER + 4];
        assert_eq!(
            parse(&text_list_with_blob(truncated, 100_000_400.0, 0.0)).err(),
            Some(ERR_FONT_TABLE)
        );

        let mut truncated = font_table(&[(100, "Inter SemiBold")]);
        let cut = FONT_TABLE_HEADER + 4;
        truncated.truncate(cut);
        put_u32(&mut truncated, 4, cut as u32);
        assert_eq!(
            parse(&text_list_with_blob(&truncated, 100_000_400.0, 0.0)).err(),
            Some(ERR_FONT_TABLE)
        );

        let duplicate = font_table(&[(100, "First"), (100, "Second")]);
        assert_eq!(
            parse(&text_list_with_blob(&duplicate, 100_000_400.0, 0.0)).err(),
            Some(ERR_FONT_TABLE)
        );

        let low_id = font_table(&[(99, "Inter SemiBold")]);
        assert_eq!(
            parse(&text_list_with_blob(&low_id, 100_000_400.0, 0.0)).err(),
            Some(ERR_FONT_TABLE)
        );

        let high_id = font_table(&[(116, "Inter SemiBold")]);
        assert_eq!(
            parse(&text_list_with_blob(&high_id, 100_000_400.0, 0.0)).err(),
            Some(ERR_FONT_TABLE)
        );

        let too_many: Vec<(u32, &str)> = (0..=FONT_TABLE_MAX_FONTS)
            .map(|index| (100 + (index % 16) as u32, "Family"))
            .collect();
        let too_many = font_table(&too_many);
        assert_eq!(
            parse(&text_list_with_blob(&too_many, 100_000_400.0, 0.0)).err(),
            Some(ERR_FONT_TABLE_SIZE)
        );
    }

    #[test]
    fn blobs_without_a_font_table_keep_generic_families() {
        let commands = parse(&text_list(400.0, 0.0)).expect("parse tableless text");
        match commands.as_slice() {
            [Command::Text { family, .. }] => assert_eq!(*family, 0),
            other => panic!("unexpected commands: {other:?}"),
        }
        let runs = shape_styled("Styled", 18.0, 400, false, 0.0, 0);
        assert!(!runs.is_empty(), "generic shaping produced no runs");
    }

    #[test]
    fn unknown_named_family_falls_back_to_sans_serif() {
        let runs = shape_styled("Unknown family probe", 18.0, 400, false, 0.0, 999);
        assert!(!runs.is_empty(), "fallback shaping produced no runs");
        assert!(
            runs.iter()
                .all(|run| run.font().name().is_none_or(|name| !name.contains("Inter"))),
            "unknown family resolved to Inter"
        );
    }

    #[test]
    fn same_id_with_different_names_does_not_reuse_cached_shapes() {
        assert_eq!(register_font(100, "Inter SemiBold", INTER), 0);
        let first = font_table(&[(100, "Inter SemiBold")]);
        parse(&text_list_with_blob(&first, 100_000_400.0, 0.0)).expect("first parse");
        let first_runs = shape_styled("Same id probe", 18.0, 400, false, 0.0, 100);
        let first_name = first_runs.iter().find_map(|run| run.font().name().map(str::to_string));
        assert!(
            first_name
                .as_deref()
                .is_some_and(|name| name.contains("Inter SemiBold")),
            "first parse did not shape with Inter SemiBold"
        );

        let second = font_table(&[(100, "Unregistered Alias")]);
        parse(&text_list_with_blob(&second, 100_000_400.0, 0.0)).expect("second parse");
        let second_runs = shape_styled("Same id probe", 18.0, 400, false, 0.0, 100);
        let second_name = second_runs
            .iter()
            .find_map(|run| run.font().name().map(str::to_string));
        assert_ne!(first_name, second_name);
        assert!(
            second_name
                .as_deref()
                .is_some_and(|name| name.contains("Unregistered Alias")),
            "second parse reused the first family name"
        );
    }

    #[test]
    fn parses_styled_text_record() {
        let commands = parse(&text_list(10700.0, 1.5)).expect("parse styled text");
        match commands.as_slice() {
            [Command::Text {
                color,
                x,
                y,
                size,
                weight,
                italic,
                tracking,
                family,
                text,
            }] => {
                assert_eq!(*color, 0xFF112233);
                assert_eq!((*x, *y), (4.0, 8.0));
                assert_eq!(*size, 18.0);
                assert_eq!(*weight, 700);
                assert!(*italic);
                assert_eq!(*tracking, 1.5);
                assert_eq!(*family, 0);
                assert_eq!(text, "Styled");
            }
            other => panic!("unexpected commands: {other:?}"),
        }
        let runs = shape_styled("Styled", 18.0, 700, true, 1.5, 0);
        assert!(!runs.is_empty(), "styled shaping produced no runs");
        let _ = to_gpu(&commands);
    }

    #[test]
    fn legacy_text_record_uses_default_style() {
        let commands = parse(&text_list(0.0, 0.0)).expect("parse legacy text");
        match commands.as_slice() {
            [Command::Text {
                weight,
                italic,
                tracking,
                family,
                ..
            }] => {
                assert_eq!(*weight, 400);
                assert!(!*italic);
                assert_eq!(*tracking, 0.0);
                assert_eq!(*family, 0);
            }
            other => panic!("unexpected commands: {other:?}"),
        }
    }

    #[test]
    fn parses_family_packed_e_lane() {
        let commands = parse(&text_list(5_000_700.0, 0.0)).expect("parse family text");
        match commands.as_slice() {
            [Command::Text {
                weight,
                italic,
                family,
                ..
            }] => {
                assert_eq!(*weight, 700);
                assert!(!*italic);
                assert_eq!(*family, 5);
            }
            other => panic!("unexpected commands: {other:?}"),
        }
    }

    #[test]
    fn legacy_italic_record_stays_sans_serif() {
        let (weight, italic, tracking, family) = text_style(10700.0, 0.5);
        assert_eq!(weight, 700);
        assert!(italic);
        assert_eq!(tracking, 0.5);
        assert_eq!(family, 0);
    }

    #[test]
    fn style_fields_change_the_shape_cache_key() {
        let regular = shape_styled("Cache key probe", 16.0, 400, false, 0.0, 0);
        let tracked = shape_styled("Cache key probe", 16.0, 400, false, 3.0, 0);
        let bold = shape_styled("Cache key probe", 16.0, 700, false, 0.0, 0);
        assert!(!regular.is_empty());
        assert!(!tracked.is_empty());
        assert!(!bold.is_empty());
    }

    #[test]
    fn family_changes_the_shape_cache_key() {
        let baseline = CACHE.with(|cache| cache.borrow().len());
        let sans = shape_styled("Family cache probe", 16.0, 400, false, 0.0, 0);
        let mono = shape_styled("Family cache probe", 16.0, 400, false, 0.0, 2);
        assert!(!sans.is_empty());
        assert!(!mono.is_empty());
        assert_eq!(CACHE.with(|cache| cache.borrow().len()), baseline + 2);
        let again = shape_styled("Family cache probe", 16.0, 400, false, 0.0, 0);
        assert_eq!(again.len(), sans.len());
        assert_eq!(CACHE.with(|cache| cache.borrow().len()), baseline + 2);
    }

    #[test]
    fn shapes_and_builds_scene_with_monospace_family() {
        let commands = parse(&text_list(2_000_400.0, 0.0)).expect("parse monospace text");
        match commands.as_slice() {
            [Command::Text { family, weight, .. }] => {
                assert_eq!(*family, 2);
                assert_eq!(*weight, 400);
            }
            other => panic!("unexpected commands: {other:?}"),
        }
        let runs = shape_styled("Styled", 18.0, 400, false, 0.0, 2);
        assert!(!runs.is_empty(), "monospace shaping produced no runs");
        let scene = to_gpu(&commands);
        assert_eq!(scene.encoding().n_open_clips, 0);
    }

    fn v3_list() -> Vec<u8> {
        let mut blob = Vec::new();
        let fill_at = blob.len();
        push_verb(&mut blob, 0, &[10.0, 10.0]);
        push_verb(&mut blob, 1, &[50.0, 10.0]);
        push_verb(&mut blob, 3, &[50.0, 50.0, 10.0, 50.0, 10.0, 10.0]);
        push_verb(&mut blob, 4, &[]);
        let stroke_at = blob.len();
        push_verb(&mut blob, 0, &[0.0, 0.0]);
        push_verb(&mut blob, 2, &[10.0, 20.0, 20.0, 0.0]);

        let blob_at = HEADER + 4 * STRIDE;
        let mut out = vec![0u8; blob_at + blob.len()];
        put_u32(&mut out, 0, MAGIC);
        put_u32(&mut out, 4, 3);
        put_u32(&mut out, 8, 4);
        put_u32(&mut out, 12, STRIDE as u32);
        put_u32(&mut out, 16, blob_at as u32);
        put_u32(&mut out, 20, blob.len() as u32);
        out[blob_at..].copy_from_slice(&blob);

        let at = HEADER;
        put_u32(&mut out, at, 11);
        put_u32(&mut out, at + 4, 0x11223344);
        put_u32(&mut out, at + 8, fill_at as u32);
        put_u32(&mut out, at + 12, 4);
        put_f64(&mut out, at + 48, 1.0);

        let at = HEADER + STRIDE;
        put_u32(&mut out, at, 12);
        put_u32(&mut out, at + 4, 0x55667788);
        put_u32(&mut out, at + 8, stroke_at as u32);
        put_u32(&mut out, at + 12, 2);
        put_f64(&mut out, at + 48, 3.5);
        put_f64(&mut out, at + 56, 9.0);

        let at = HEADER + 2 * STRIDE;
        put_u32(&mut out, at, 13);
        put_f64(&mut out, at + 16, 4.0);
        put_f64(&mut out, at + 24, 6.0);
        put_f64(&mut out, at + 32, 100.0);
        put_f64(&mut out, at + 40, 80.0);
        put_f64(&mut out, at + 48, 0.5);
        put_f64(&mut out, at + 56, 0.0);

        put_u32(&mut out, HEADER + 3 * STRIDE, 14);
        out
    }

    #[test]
    fn parses_v3_paths_and_layers() {
        use vello::kurbo::PathEl;

        let commands = parse(&v3_list()).expect("parse v3 list");
        assert_eq!(commands.len(), 4);

        match &commands[0] {
            Command::PathFill {
                color,
                path,
                even_odd,
            } => {
                assert_eq!(*color, 0x11223344);
                assert!(*even_odd);
                let elements: Vec<PathEl> = path.elements().to_vec();
                assert_eq!(
                    elements,
                    vec![
                        PathEl::MoveTo(Point::new(10.0, 10.0)),
                        PathEl::LineTo(Point::new(50.0, 10.0)),
                        PathEl::CurveTo(
                            Point::new(50.0, 50.0),
                            Point::new(10.0, 50.0),
                            Point::new(10.0, 10.0),
                        ),
                        PathEl::ClosePath,
                    ]
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match &commands[1] {
            Command::PathStroke {
                color,
                path,
                width,
                cap,
                join,
            } => {
                assert_eq!(*color, 0x55667788);
                assert_eq!(*width, 3.5);
                assert_eq!((*cap, *join), (1, 2));
                let elements: Vec<PathEl> = path.elements().to_vec();
                assert_eq!(
                    elements,
                    vec![
                        PathEl::MoveTo(Point::new(0.0, 0.0)),
                        PathEl::QuadTo(Point::new(10.0, 20.0), Point::new(20.0, 0.0)),
                    ]
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match &commands[2] {
            Command::LayerPush {
                x,
                y,
                w,
                h,
                alpha,
                blend,
            } => {
                assert_eq!((*x, *y, *w, *h), (4.0, 6.0, 100.0, 80.0));
                assert_eq!(*alpha, 0.5);
                assert_eq!(*blend, 0);
            }
            other => panic!("unexpected command: {other:?}"),
        }
        assert!(matches!(commands[3], Command::LayerPop));

        let _ = to_gpu(&commands);

        let mut broken = v3_list();
        put_u32(&mut broken, HEADER + 12, 99);
        assert_eq!(parse(&broken).err(), Some(-21));
    }

    fn shadow_and_clip_list() -> Vec<u8> {
        let blob_at = HEADER + 3 * STRIDE;
        let mut out = vec![0u8; blob_at];
        put_u32(&mut out, 0, MAGIC);
        put_u32(&mut out, 4, 3);
        put_u32(&mut out, 8, 3);
        put_u32(&mut out, 12, STRIDE as u32);
        put_u32(&mut out, 16, blob_at as u32);
        put_u32(&mut out, 20, 0);

        let at = HEADER;
        put_u32(&mut out, at, 17);
        put_u32(&mut out, at + 4, 0x10203040);
        put_f64(&mut out, at + 16, 5.0);
        put_f64(&mut out, at + 24, 6.0);
        put_f64(&mut out, at + 32, 70.0);
        put_f64(&mut out, at + 40, 80.0);
        put_f64(&mut out, at + 48, 12.0);
        put_f64(&mut out, at + 56, 2.5);

        let at = HEADER + STRIDE;
        put_u32(&mut out, at, 18);
        put_f64(&mut out, at + 16, 1.0);
        put_f64(&mut out, at + 24, 2.0);
        put_f64(&mut out, at + 32, 30.0);
        put_f64(&mut out, at + 40, 40.0);
        put_f64(&mut out, at + 48, 8.0);

        put_u32(&mut out, HEADER + 2 * STRIDE, 4);
        out
    }

    #[test]
    fn parses_shadow_and_rounded_clip() {
        let commands = parse(&shadow_and_clip_list()).expect("parse shadow list");
        assert_eq!(commands.len(), 3);

        match &commands[0] {
            Command::RoundShadow {
                color,
                x,
                y,
                w,
                h,
                radius,
                std_dev,
            } => {
                assert_eq!(*color, 0x10203040);
                assert_eq!((*x, *y, *w, *h), (5.0, 6.0, 70.0, 80.0));
                assert_eq!((*radius, *std_dev), (12.0, 2.5));
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match &commands[1] {
            Command::ClipPushRound { x, y, w, h, radius } => {
                assert_eq!((*x, *y, *w, *h, *radius), (1.0, 2.0, 30.0, 40.0, 8.0));
            }
            other => panic!("unexpected command: {other:?}"),
        }

        assert!(matches!(commands[2], Command::ClipPop));
        let _ = to_gpu(&commands);
    }

    #[test]
    fn parses_layer_blend_ids() {
        let mut bytes = v3_list();
        let at = HEADER + 2 * STRIDE;
        put_f64(&mut bytes, at + 56, 6.0);
        let commands = parse(&bytes).expect("parse blended layer");
        match &commands[2] {
            Command::LayerPush { blend, alpha, .. } => {
                assert_eq!(*blend, 6);
                assert_eq!(*alpha, 0.5);
            }
            other => panic!("unexpected command: {other:?}"),
        }
        assert_eq!(blend_of(0), BlendMode::default());
        assert_eq!(blend_of(1), Mix::Multiply.into());
        assert_eq!(blend_of(6), Mix::Difference.into());
        assert_eq!(blend_of(8), BlendMode::new(Mix::Normal, Compose::Plus));
        assert_eq!(blend_of(200), BlendMode::default());
        let _ = to_gpu(&commands);
    }

    fn tiny_png(rgb: [u8; 3]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, 2, 2);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("png header");
            let mut pixels = Vec::with_capacity(2 * 2 * 4);
            for _ in 0..4 {
                pixels.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            }
            writer.write_image_data(&pixels).expect("png data");
        }
        out
    }

    const TINY_JPEG: [u8; 285] = [
        255, 216, 255, 224, 0, 16, 74, 70, 73, 70, 0, 1, 1, 0, 0, 1, 0, 1, 0, 0, 255, 219, 0, 67,
        0, 2, 1, 1, 1, 1, 1, 2, 1, 1, 1, 2, 2, 2, 2, 2, 4, 3, 2, 2, 2, 2, 5, 4, 4, 3, 4, 6, 5, 6,
        6, 6, 5, 6, 6, 6, 7, 9, 8, 6, 7, 9, 7, 6, 6, 8, 11, 8, 9, 10, 10, 10, 10, 10, 6, 8, 11, 12,
        11, 10, 12, 9, 10, 10, 10, 255, 219, 0, 67, 1, 2, 2, 2, 2, 2, 2, 5, 3, 3, 5, 10, 7, 6, 7,
        10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10,
        10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10,
        10, 10, 10, 10, 255, 192, 0, 17, 8, 0, 2, 0, 2, 3, 1, 17, 0, 2, 17, 1, 3, 17, 1, 255, 196,
        0, 20, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 9, 255, 196, 0, 20, 16, 1, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 255, 196, 0, 20, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 5, 255, 196, 0, 20, 17, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 255, 218, 0, 12, 3, 1, 0, 2, 17, 3, 17, 0, 63, 0, 17, 206, 7, 127, 255, 217,
    ];

    const TINY_JPEG_GRAY: [u8; 159] = [
        255, 216, 255, 224, 0, 16, 74, 70, 73, 70, 0, 1, 1, 0, 0, 1, 0, 1, 0, 0, 255, 219, 0, 67,
        0, 2, 1, 1, 1, 1, 1, 2, 1, 1, 1, 2, 2, 2, 2, 2, 4, 3, 2, 2, 2, 2, 5, 4, 4, 3, 4, 6, 5, 6,
        6, 6, 5, 6, 6, 6, 7, 9, 8, 6, 7, 9, 7, 6, 6, 8, 11, 8, 9, 10, 10, 10, 10, 10, 6, 8, 11, 12,
        11, 10, 12, 9, 10, 10, 10, 255, 192, 0, 11, 8, 0, 2, 0, 2, 1, 1, 17, 0, 255, 196, 0, 20, 0,
        1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 255, 196, 0, 20, 16, 1, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 255, 218, 0, 8, 1, 1, 0, 0, 63, 0, 63, 255, 217,
    ];

    const TINY_WEBP: [u8; 32] = [
        82, 73, 70, 70, 24, 0, 0, 0, 87, 69, 66, 80, 86, 80, 56, 76, 12, 0, 0, 0, 47, 1, 64, 0, 0,
        40, 69, 21, 234, 209, 255, 0,
    ];

    const TINY_WEBP_ALPHA: [u8; 32] = [
        82, 73, 70, 70, 24, 0, 0, 0, 87, 69, 66, 80, 86, 80, 56, 76, 12, 0, 0, 0, 47, 1, 64, 0, 16,
        40, 69, 21, 234, 81, 192, 0,
    ];

    fn rgba_pixel(image: &ImageData, x: usize, y: usize) -> [u8; 4] {
        let at = (y * image.width as usize + x) * 4;
        image.data.data()[at..at + 4]
            .try_into()
            .expect("rgba pixel")
    }

    #[test]
    fn decodes_raw_rgba_media_frames() {
        let mut frame = vec![0u8; 24 + 2 * 2 * 4];
        put_u32(&mut frame, 0, 0x5258_4E52);
        put_u32(&mut frame, 4, 2);
        put_u32(&mut frame, 8, 2);
        put_u32(&mut frame, 12, 8);
        put_u32(&mut frame, 16, 0);
        put_u32(&mut frame, 20, 16);
        frame[24..].copy_from_slice(&[
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 128,
        ]);
        assert!(is_raw_frame(&frame));
        let image = decode_image_at(&frame, 0, 0).expect("raw frame");
        assert_eq!((image.width, image.height), (2, 2));
        assert_eq!(image.format, ImageFormat::Rgba8);
        assert_eq!(image.alpha_type, ImageAlphaType::Alpha);
        assert_eq!(rgba_pixel(&image, 0, 0), [255, 0, 0, 255]);
        assert_eq!(rgba_pixel(&image, 2 % 2, 1), [0, 0, 255, 255]);

        let mut bad = frame.clone();
        put_u32(&mut bad, 12, 16);
        assert!(decode_image_at(&bad, 0, 0).is_none());
        let mut short = frame.clone();
        short.truncate(24 + 8);
        assert!(decode_image_at(&short, 0, 0).is_none());
    }

    #[test]
    fn png_cache_hits_on_second_decode() {
        reset_image_cache();
        let png = tiny_png([10, 20, 30]);
        let first = decode_png(&png).expect("decode first");
        assert_eq!(png_cache_stats(), (0, 1, 1, 16));
        let second = decode_png(&png).expect("decode second");
        assert_eq!(png_cache_stats(), (1, 1, 1, 16));
        assert_eq!(first.data.data(), second.data.data());
    }

    #[test]
    fn png_cache_keeps_distinct_images_apart() {
        reset_image_cache();
        let a = tiny_png([1, 2, 3]);
        let b = tiny_png([4, 5, 6]);
        let image_a = decode_png(&a).expect("decode a");
        let image_b = decode_png(&b).expect("decode b");
        assert_eq!(png_cache_stats(), (0, 2, 2, 32));
        assert_ne!(image_a.data.data(), image_b.data.data());
    }

    #[test]
    fn png_cache_does_not_cache_failures() {
        reset_image_cache();
        let corrupt = b"not a png at all";
        assert!(decode_png(corrupt).is_none());
        assert_eq!(png_cache_stats(), (0, 1, 0, 0));
        assert!(decode_png(corrupt).is_none());
        assert_eq!(png_cache_stats(), (0, 2, 0, 0));
    }

    #[test]
    fn png_cache_is_bounded() {
        reset_image_cache();
        let count = CACHE_MAX_ENTRIES as u8 + 10;
        for value in 0..count {
            let png = tiny_png([value, value, value]);
            assert!(decode_png(&png).is_some());
        }
        assert_eq!(
            png_cache_stats(),
            (0, count as u64, CACHE_MAX_ENTRIES, CACHE_MAX_ENTRIES * 16)
        );
    }

    #[test]
    fn png_cache_is_thread_local() {
        reset_image_cache();
        let png = tiny_png([9, 9, 9]);
        assert!(decode_png(&png).is_some());
        let other = std::thread::spawn(|| {
            let png = tiny_png([9, 9, 9]);
            assert!(decode_png(&png).is_some());
            png_cache_stats()
        })
        .join()
        .expect("cache thread");
        assert_eq!(other, (0, 1, 1, 16));
        assert_eq!(png_cache_stats(), (0, 1, 1, 16));
    }

    #[test]
    fn decodes_png_jpeg_and_webp_to_rgba8() {
        reset_image_cache();
        let png = tiny_png([10, 20, 30]);
        let image = decode_png(&png).expect("decode png");
        assert_eq!((image.width, image.height), (2, 2));
        assert_eq!(rgba_pixel(&image, 0, 0), [10, 20, 30, 255]);

        let image = decode_png(&TINY_JPEG).expect("decode jpeg");
        assert_eq!((image.width, image.height), (2, 2));
        let pixel = rgba_pixel(&image, 0, 0);
        assert!((pixel[0] as i16 - 10).abs() <= 4, "jpeg red {pixel:?}");
        assert!((pixel[1] as i16 - 20).abs() <= 4, "jpeg green {pixel:?}");
        assert!((pixel[2] as i16 - 30).abs() <= 4, "jpeg blue {pixel:?}");
        assert_eq!(pixel[3], 255);

        let image = decode_png(&TINY_WEBP).expect("decode webp");
        assert_eq!((image.width, image.height), (2, 2));
        assert_eq!(rgba_pixel(&image, 1, 1), [10, 20, 30, 255]);

        let image = decode_png(&TINY_WEBP_ALPHA).expect("decode webp alpha");
        assert_eq!((image.width, image.height), (2, 2));
        assert_eq!(rgba_pixel(&image, 1, 1), [10, 20, 30, 128]);
    }

    #[test]
    fn decodes_grayscale_jpeg_to_rgba8() {
        reset_image_cache();
        let image = decode_png(&TINY_JPEG_GRAY).expect("decode gray jpeg");
        assert_eq!((image.width, image.height), (2, 2));
        let pixel = rgba_pixel(&image, 0, 0);
        assert!((pixel[0] as i16 - 128).abs() <= 3, "gray {pixel:?}");
        assert_eq!(pixel[0], pixel[1]);
        assert_eq!(pixel[1], pixel[2]);
        assert_eq!(pixel[3], 255);
    }

    #[test]
    fn image_cache_hits_for_every_format() {
        reset_image_cache();
        let png = tiny_png([10, 20, 30]);
        for bytes in [&png[..], &TINY_JPEG[..], &TINY_WEBP[..]] {
            assert!(decode_png(bytes).is_some());
        }
        assert_eq!(png_cache_stats(), (0, 3, 3, 48));
        for bytes in [&png[..], &TINY_JPEG[..], &TINY_WEBP[..]] {
            assert!(decode_png(bytes).is_some());
        }
        assert_eq!(png_cache_stats(), (3, 3, 3, 48));
    }

    #[test]
    fn image_cache_misses_on_different_bytes() {
        reset_image_cache();
        let png_a = tiny_png([1, 2, 3]);
        let png_b = tiny_png([4, 5, 6]);
        for bytes in [
            &png_a[..],
            &png_b[..],
            &TINY_JPEG[..],
            &TINY_JPEG_GRAY[..],
            &TINY_WEBP[..],
            &TINY_WEBP_ALPHA[..],
        ] {
            assert!(decode_png(bytes).is_some());
        }
        assert_eq!(png_cache_stats(), (0, 6, 6, 96));
    }

    #[test]
    fn corrupt_images_fail_without_caching() {
        reset_image_cache();
        let png = tiny_png([7, 8, 9]);
        let corrupt_png = &png[..8];
        let corrupt_jpeg: [u8; 4] = [0xff, 0xd8, 0xff, 0xd9];
        let corrupt_webp = b"RIFF\x10\x00\x00\x00WEBPjunk";
        let corrupt = [corrupt_png, &corrupt_jpeg[..], &corrupt_webp[..]];
        for bytes in corrupt {
            assert!(decode_png(bytes).is_none());
        }
        assert_eq!(png_cache_stats(), (0, 3, 0, 0));
        for bytes in corrupt {
            assert!(decode_png(bytes).is_none());
        }
        assert_eq!(png_cache_stats(), (0, 6, 0, 0));
    }

    const TINY_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="2" height="2" fill="#0a141e"/></svg>"##;

    #[test]
    fn decodes_svg_at_intrinsic_and_scaled_size() {
        reset_svg_cache();
        let intrinsic = decode_image_at(TINY_SVG, 0, 0).expect("intrinsic svg");
        assert_eq!((intrinsic.width, intrinsic.height), (2, 2));
        assert_eq!(rgba_pixel(&intrinsic, 0, 0), [10, 20, 30, 255]);
        assert_eq!(intrinsic.alpha_type, ImageAlphaType::AlphaPremultiplied);
        assert_eq!(intrinsic.format, ImageFormat::Rgba8);

        let scaled = decode_image_at(TINY_SVG, 4, 4).expect("scaled svg");
        assert_eq!((scaled.width, scaled.height), (4, 4));
        assert_eq!(rgba_pixel(&scaled, 3, 3), [10, 20, 30, 255]);
        assert_eq!(rgba_pixel(&scaled, 0, 0), [10, 20, 30, 255]);

        assert_eq!(SVG_CACHE.with(|cache| cache.borrow().entries.len()), 2);
        let cached = decode_image_at(TINY_SVG, 4, 4).expect("cached svg");
        assert_eq!(cached.data.data(), scaled.data.data());
        assert_eq!(SVG_CACHE.with(|cache| cache.borrow().entries.len()), 2);
    }

    #[test]
    fn malformed_and_oversized_svg_fail_without_caching() {
        reset_svg_cache();
        let truncated =
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="2""#;
        assert!(decode_image_at(truncated, 0, 0).is_none());
        assert!(decode_image_at(truncated, 0, 0).is_none());
        assert_eq!(SVG_CACHE.with(|cache| cache.borrow().entries.len()), 0);

        let zero = br#"<svg xmlns="http://www.w3.org/2000/svg" width="0" height="0"></svg>"#;
        assert!(decode_image_at(zero, 0, 0).is_none());
        assert_eq!(SVG_CACHE.with(|cache| cache.borrow().entries.len()), 0);

        assert!(decode_image_at(TINY_SVG, 1_000_000, 1_000_000).is_none());
        assert!(decode_image_at(TINY_SVG, SVG_MAX_TARGET_SIDE + 1, 1).is_none());
        assert_eq!(SVG_CACHE.with(|cache| cache.borrow().entries.len()), 0);
    }

    #[test]
    fn raster_images_ignore_the_target_and_keep_the_cache_key() {
        reset_image_cache();
        let png = tiny_png([1, 2, 3]);
        let image = decode_image_at(&png, 32, 32).expect("decode raster");
        assert_eq!((image.width, image.height), (2, 2));
        assert_eq!(rgba_pixel(&image, 0, 0), [1, 2, 3, 255]);
        assert_eq!(png_cache_stats(), (0, 1, 1, 16));
        assert!(decode_png(&png).is_some());
        assert_eq!(png_cache_stats(), (1, 1, 1, 16));
    }

    fn effect_list() -> Vec<u8> {
        let records = 6;
        let blob_at = HEADER + records * STRIDE;
        let mut out = vec![0u8; blob_at];
        put_u32(&mut out, 0, MAGIC);
        put_u32(&mut out, 4, 3);
        put_u32(&mut out, 8, records as u32);
        put_u32(&mut out, 12, STRIDE as u32);
        put_u32(&mut out, 16, blob_at as u32);
        put_u32(&mut out, 20, 0);

        let at = HEADER;
        put_u32(&mut out, at, 19);
        put_f64(&mut out, at + 16, 4.0);
        put_f64(&mut out, at + 24, 6.0);
        put_f64(&mut out, at + 32, 40.0);
        put_f64(&mut out, at + 40, 30.0);
        put_f64(&mut out, at + 48, 8.0);

        let at = HEADER + STRIDE;
        put_u32(&mut out, at, 1);
        put_u32(&mut out, at + 4, 0xFF00_00FF);
        put_f64(&mut out, at + 16, 4.0);
        put_f64(&mut out, at + 24, 6.0);
        put_f64(&mut out, at + 32, 40.0);
        put_f64(&mut out, at + 40, 30.0);

        put_u32(&mut out, HEADER + 2 * STRIDE, 20);

        let at = HEADER + 3 * STRIDE;
        put_u32(&mut out, at, 21);
        put_f64(&mut out, at + 16, 0.0);
        put_f64(&mut out, at + 24, 0.0);
        put_f64(&mut out, at + 32, 64.0);
        put_f64(&mut out, at + 40, 48.0);
        put_f64(&mut out, at + 48, 3.5);

        let at = HEADER + 4 * STRIDE;
        put_u32(&mut out, at, 1);
        put_u32(&mut out, at + 4, 0x00FF_00FF);
        put_f64(&mut out, at + 16, 0.0);
        put_f64(&mut out, at + 24, 0.0);
        put_f64(&mut out, at + 32, 64.0);
        put_f64(&mut out, at + 40, 48.0);

        put_u32(&mut out, HEADER + 5 * STRIDE, 22);
        out
    }

    #[test]
    fn parses_mask_and_blur_records() {
        let commands = parse(&effect_list()).expect("parse mask and blur");
        assert_eq!(commands.len(), 6);

        match &commands[0] {
            Command::PushMask { x, y, w, h, radius } => {
                assert_eq!((*x, *y, *w, *h, *radius), (4.0, 6.0, 40.0, 30.0, 8.0))
            }
            other => panic!("unexpected command: {other:?}"),
        }
        assert!(matches!(commands[1], Command::Fill { .. }));
        assert!(matches!(commands[2], Command::PopMask));

        match &commands[3] {
            Command::PushBlur { x, y, w, h, sigma } => {
                assert_eq!((*x, *y, *w, *h, *sigma), (0.0, 0.0, 64.0, 48.0, 3.5));
            }
            other => panic!("unexpected command: {other:?}"),
        }
        assert!(matches!(commands[4], Command::Fill { .. }));
        assert!(matches!(commands[5], Command::PopBlur));

        let scene = to_gpu(&commands);
        assert_eq!(scene.encoding().n_open_clips, 0);
        assert!(scene.encoding().n_clips >= 1);
    }

    fn pop_only_list() -> Vec<u8> {
        let records = 4;
        let mut out = vec![0u8; HEADER + records * STRIDE];
        put_u32(&mut out, 0, MAGIC);
        put_u32(&mut out, 4, 3);
        put_u32(&mut out, 8, records as u32);
        put_u32(&mut out, 12, STRIDE as u32);
        put_u32(&mut out, 16, (HEADER + records * STRIDE) as u32);
        put_u32(&mut out, 20, 0);
        put_u32(&mut out, HEADER, 20);
        put_u32(&mut out, HEADER + STRIDE, 22);
        put_u32(&mut out, HEADER + 2 * STRIDE, 20);
        put_u32(&mut out, HEADER + 3 * STRIDE, 22);
        out
    }

    #[test]
    fn unbalanced_mask_and_blur_pops_are_noops() {
        let commands = parse(&pop_only_list()).expect("parse unbalanced pops");
        assert!(matches!(commands[0], Command::PopMask));
        assert!(matches!(commands[1], Command::PopBlur));
        assert!(matches!(commands[2], Command::PopMask));
        assert!(matches!(commands[3], Command::PopBlur));

        let scene = to_gpu(&commands);
        assert_eq!(scene.encoding().n_open_clips, 0);
        assert_eq!(scene.encoding().n_clips, 0);
    }

    fn masked_child_list() -> Vec<u8> {
        let records = 3;
        let mut out = vec![0u8; HEADER + records * STRIDE];
        put_u32(&mut out, 0, MAGIC);
        put_u32(&mut out, 4, 3);
        put_u32(&mut out, 8, records as u32);
        put_u32(&mut out, 12, STRIDE as u32);
        put_u32(&mut out, 16, (HEADER + records * STRIDE) as u32);
        put_u32(&mut out, 20, 0);

        let at = HEADER;
        put_u32(&mut out, at, 19);
        put_f64(&mut out, at + 16, 2.0);
        put_f64(&mut out, at + 24, 3.0);
        put_f64(&mut out, at + 32, 60.0);
        put_f64(&mut out, at + 40, 50.0);
        put_f64(&mut out, at + 48, 10.0);

        let at = HEADER + STRIDE;
        put_u32(&mut out, at, 1);
        put_u32(&mut out, at + 4, 0x3366_CCFF);
        put_f64(&mut out, at + 16, 0.0);
        put_f64(&mut out, at + 24, 0.0);
        put_f64(&mut out, at + 32, 64.0);
        put_f64(&mut out, at + 40, 64.0);

        put_u32(&mut out, HEADER + 2 * STRIDE, 20);
        out
    }

    #[test]
    fn builds_scene_with_masked_child() {
        let commands = parse(&masked_child_list()).expect("parse masked child");
        let scene = to_gpu(&commands);
        let encoding = scene.encoding();
        assert_eq!(encoding.n_open_clips, 0);
        assert!(encoding.n_clips >= 1);
        assert!(encoding.n_paths >= 2, "expected a clip path and a fill");
    }

    fn gradient_rect_list(kind: f64, radius: f64, stops: &[(u32, f64)]) -> Vec<u8> {
        let blob_at = HEADER + STRIDE;
        let blob_len = stops.len() * 8;
        let mut out = vec![0u8; blob_at + blob_len];
        put_u32(&mut out, 0, MAGIC);
        put_u32(&mut out, 4, 3);
        put_u32(&mut out, 8, 1);
        put_u32(&mut out, 12, STRIDE as u32);
        put_u32(&mut out, 16, blob_at as u32);
        put_u32(&mut out, 20, blob_len as u32);
        put_u32(&mut out, 24, 64);
        put_u32(&mut out, 28, 64);

        let at = HEADER;
        put_u32(&mut out, at, 25);
        put_u32(&mut out, at + 8, 0);
        put_u32(&mut out, at + 12, stops.len() as u32);
        put_f64(&mut out, at + 16, 0.0);
        put_f64(&mut out, at + 24, 0.0);
        put_f64(&mut out, at + 32, 64.0);
        put_f64(&mut out, at + 40, 64.0);
        put_f64(&mut out, at + 48, kind);
        put_f64(&mut out, at + 56, radius);

        for (i, &(color, offset)) in stops.iter().enumerate() {
            let at = blob_at + i * 8;
            put_u32(&mut out, at, color);
            put_f32(&mut out, at + 4, offset as f32);
        }
        out
    }

    fn stop_rgba(stop: &ColorStop) -> [u8; 4] {
        stop.color
            .to_alpha_color::<vello::peniko::color::Srgb>()
            .to_rgba8()
            .to_u8_array()
    }

    const RED: u32 = 0xFF00_00FF;
    const GREEN: u32 = 0x00FF_00FF;
    const BLUE: u32 = 0x0000_FFFF;

    #[test]
    fn parses_gradient_rect_and_clamps_offsets() {
        let commands = parse(&gradient_rect_list(
            1.0,
            0.0,
            &[(RED, 1.5), (GREEN, 0.5), (BLUE, -0.5)],
        ))
        .expect("parse gradient rect");
        match commands.as_slice() {
            [Command::GradientRect {
                x,
                y,
                w,
                h,
                kind,
                radius,
                stops,
            }] => {
                assert_eq!((*x, *y, *w, *h), (0.0, 0.0, 64.0, 64.0));
                assert_eq!(*kind, 1);
                assert_eq!(*radius, 0.0);
                assert_eq!(stops.as_slice(), &[(BLUE, 0.0), (GREEN, 0.5), (RED, 1.0)]);
            }
            other => panic!("unexpected commands: {other:?}"),
        }
        let _ = to_gpu(&commands);
    }

    #[test]
    fn three_stop_horizontal_gradient_has_a_distinct_mid_color() {
        let stops = [(RED, 0.0), (GREEN, 0.5), (BLUE, 1.0)];
        let commands = parse(&gradient_rect_list(1.0, 0.0, &stops)).expect("parse three stops");
        let brush = gradient_brush(
            1,
            0.0,
            0.0,
            64.0,
            32.0,
            0.0,
            &stops.map(|(color, offset)| (color, offset as f32)),
        );
        match &brush.kind {
            GradientKind::Linear(LinearGradientPosition { start, end }) => {
                assert_eq!(*start, Point::new(0.0, 0.0));
                assert_eq!(*end, Point::new(64.0, 0.0));
            }
            other => panic!("unexpected gradient kind: {other:?}"),
        }
        assert_eq!(brush.stops.len(), 3);
        assert_eq!(
            [
                stop_rgba(&brush.stops[0]),
                stop_rgba(&brush.stops[1]),
                stop_rgba(&brush.stops[2])
            ],
            [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]]
        );

        let scene = to_gpu(&commands);
        let encoded = &scene.encoding().resources.color_stops;
        assert_eq!(encoded.len(), 3);
        let mid = stop_rgba(&encoded[1]);
        assert_ne!(mid, stop_rgba(&encoded[0]));
        assert_ne!(mid, stop_rgba(&encoded[2]));
    }

    #[test]
    fn radial_gradient_center_differs_from_edge() {
        let stops = [(0xFFFF_FFFF, 0.0), (0x0000_00FF, 1.0)];
        let brush = gradient_brush(2, 10.0, 20.0, 40.0, 20.0, 0.0, &stops);
        match &brush.kind {
            GradientKind::Radial(position) => {
                assert_eq!(position.start_center, Point::new(30.0, 30.0));
                assert_eq!(position.end_center, Point::new(30.0, 30.0));
                assert_eq!(position.start_radius, 0.0);
                assert_eq!(position.end_radius, 10.0);
            }
            other => panic!("unexpected gradient kind: {other:?}"),
        }
        assert_ne!(stop_rgba(&brush.stops[0]), stop_rgba(&brush.stops[1]));

        let explicit = gradient_brush(2, 10.0, 20.0, 40.0, 20.0, 7.0, &stops);
        match &explicit.kind {
            GradientKind::Radial(position) => assert_eq!(position.end_radius, 7.0),
            other => panic!("unexpected gradient kind: {other:?}"),
        }

        let commands = parse(&gradient_rect_list(
            2.0,
            0.0,
            &[(0xFFFF_FFFF, 0.0), (0x0000_00FF, 1.0)],
        ))
        .expect("parse radial");
        match commands.as_slice() {
            [Command::GradientRect { kind, stops, .. }] => {
                assert_eq!(*kind, 2);
                assert_eq!(stops.len(), 2);
            }
            other => panic!("unexpected commands: {other:?}"),
        }
        let _ = to_gpu(&commands);
    }

    #[test]
    fn rejects_malformed_gradient_stop_blobs() {
        let too_few = gradient_rect_list(0.0, 0.0, &[(RED, 0.0)]);
        assert_eq!(parse(&too_few).err(), Some(-20));

        let mut truncated = gradient_rect_list(0.0, 0.0, &[(RED, 0.0), (GREEN, 1.0)]);
        put_u32(&mut truncated, 20, 8);
        assert_eq!(parse(&truncated).err(), Some(-20));

        let mut non_finite = gradient_rect_list(0.0, 0.0, &[(RED, 0.0), (GREEN, 1.0)]);
        put_u32(&mut non_finite, HEADER + STRIDE + 12, f32::NAN.to_bits());
        assert_eq!(parse(&non_finite).err(), Some(-20));

        let mut overlong = gradient_rect_list(0.0, 0.0, &[(RED, 0.0), (GREEN, 1.0)]);
        put_u32(&mut overlong, HEADER + 12, u32::MAX);
        assert_eq!(parse(&overlong).err(), Some(-20));
    }

    fn blur_body_commands() -> Vec<Command> {
        vec![
            Command::PushBlur {
                x: 0.0,
                y: 0.0,
                w: 64.0,
                h: 64.0,
                sigma: 2.0,
            },
            Command::Fill {
                color: 0xFF00_00FF,
                x: 4.0,
                y: 4.0,
                w: 8.0,
                h: 8.0,
            },
            Command::Fill {
                color: 0x00FF_00FF,
                x: 16.0,
                y: 4.0,
                w: 8.0,
                h: 8.0,
            },
            Command::Fill {
                color: 0x0000_FFFF,
                x: 4.0,
                y: 16.0,
                w: 8.0,
                h: 8.0,
            },
            Command::PopBlur,
        ]
    }

    #[test]
    fn collects_blur_spec_with_rect_sigma_and_transform() {
        let commands = vec![
            Command::Fill {
                color: 0,
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
            },
            Command::TransformPush([2.0, 0.0, 0.0, 1.0, 10.0, 20.0]),
            Command::PushBlur {
                x: 1.0,
                y: 2.0,
                w: 30.0,
                h: 40.0,
                sigma: 2.5,
            },
            Command::Fill {
                color: 0,
                x: 3.0,
                y: 4.0,
                w: 5.0,
                h: 6.0,
            },
            Command::PopBlur,
            Command::TransformPop,
        ];
        let specs = collect_blurs(&commands, Affine::translate((5.0, 6.0)));
        assert_eq!(specs.len(), 1);
        let spec = &specs[0];
        assert_eq!((spec.start, spec.end), (3, 4));
        assert_eq!(
            (spec.x, spec.y, spec.w, spec.h, spec.sigma),
            (1.0, 2.0, 30.0, 40.0, 2.5)
        );
        let expected = Affine::translate((5.0, 6.0)) * Affine::new([2.0, 0.0, 0.0, 1.0, 10.0, 20.0]);
        assert_eq!(spec.transform, expected);
    }

    #[test]
    fn nested_blurs_stay_inside_the_parent_body() {
        let commands = vec![
            Command::PushBlur {
                x: 0.0,
                y: 0.0,
                w: 64.0,
                h: 64.0,
                sigma: 4.0,
            },
            Command::PushBlur {
                x: 8.0,
                y: 8.0,
                w: 16.0,
                h: 16.0,
                sigma: 1.0,
            },
            Command::Fill {
                color: 0xFF00_00FF,
                x: 8.0,
                y: 8.0,
                w: 16.0,
                h: 16.0,
            },
            Command::PopBlur,
            Command::PopBlur,
        ];
        let specs = collect_blurs(&commands, Affine::IDENTITY);
        assert_eq!(specs.len(), 1);
        assert_eq!((specs[0].start, specs[0].end), (1, 4));
        let body = &commands[specs[0].start..specs[0].end];
        assert!(matches!(body[0], Command::PushBlur { .. }));

        let nested = collect_blurs(body, specs[0].transform);
        assert_eq!(nested.len(), 1);
        assert_eq!((nested[0].start, nested[0].end), (1, 2));
    }

    #[test]
    fn images_replace_blur_bodies_and_missing_images_fall_back() {
        let commands = blur_body_commands();
        assert_eq!(to_gpu(&commands).encoding().n_paths, 3);

        let image = ImageData {
            data: Blob::new(Arc::new(vec![255u8, 0, 0, 255])),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::AlphaPremultiplied,
            width: 1,
            height: 1,
        };
        let scene = to_gpu_images(&commands, Affine::IDENTITY, &[image]);
        assert_eq!(scene.encoding().n_paths, 1);
        assert_eq!(scene.encoding().n_open_clips, 0);

        let fallback = to_gpu_images(&commands, Affine::IDENTITY, &[]);
        assert_eq!(fallback.encoding().n_paths, 3);
        assert_eq!(fallback.encoding().n_open_clips, 0);
    }
}
