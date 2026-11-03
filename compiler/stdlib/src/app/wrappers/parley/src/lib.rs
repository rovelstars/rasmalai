use std::cell::RefCell;
use std::ffi::c_void;

thread_local! {
    static FONTS: RefCell<Option<parley::FontContext>> = const { RefCell::new(None) };
}

fn build_layout(text: &str, size: f32, max_width: Option<f32>) -> parley::Layout<()> {
    FONTS.with(|slot| {
        let mut slot = slot.borrow_mut();
        let font_cx = slot.get_or_insert_with(parley::FontContext::new);
        let mut layout_cx: parley::LayoutContext<()> = parley::LayoutContext::new();
        let mut builder = layout_cx.ranged_builder(font_cx, text, 1.0, true);
        builder.push_default(parley::StyleProperty::FontSize(size));
        let mut layout = builder.build(text);
        layout.break_all_lines(max_width);
        layout
    })
}

fn text_width(text: &str, size: f32) -> f32 {
    build_layout(text, size, None).width()
}

const ROW_BYTES: usize = 24;
const ELLIPSIS: &str = "…";

fn fit_ellipsis(
    text: &str,
    start: usize,
    end: usize,
    size: f32,
    max_width: f32,
) -> (usize, f32) {
    let ellipsis_width = text_width(ELLIPSIS, size);
    if max_width <= 0.0 {
        return (end, text_width(&text[start..end], size) + ellipsis_width);
    }
    if text_width(&text[start..end], size) + ellipsis_width <= max_width {
        return (end, text_width(&text[start..end], size) + ellipsis_width);
    }
    if ellipsis_width > max_width {
        return (start, ellipsis_width);
    }
    let starts: Vec<usize> = text[start..end]
        .char_indices()
        .map(|(i, _)| start + i)
        .collect();
    let mut lo = 0usize;
    let mut hi = starts.len();
    let mut best = (start, ellipsis_width);
    while lo < hi {
        let mid = (lo + hi) / 2;
        let cut = starts[mid];
        let mut candidate = String::with_capacity(cut - start + ELLIPSIS.len());
        candidate.push_str(&text[start..cut]);
        candidate.push_str(ELLIPSIS);
        let w = text_width(&candidate, size);
        if w <= max_width {
            best = (cut, w);
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    best
}

#[no_mangle]
pub extern "C" fn parley_measure(text: *const u8, len: i32, size: f64) -> f64 {
    if text.is_null() || len <= 0 || size <= 0.0 {
        return -1.0;
    }
    let bytes = unsafe { std::slice::from_raw_parts(text, len as usize) };
    let Ok(s) = std::str::from_utf8(bytes) else {
        return -2.0;
    };
    text_width(s, size as f32) as f64
}

fn char_to_byte(text: &str, at: i32) -> usize {
    if at <= 0 {
        return 0;
    }
    text.char_indices()
        .nth(at as usize)
        .map(|(b, _)| b)
        .unwrap_or(text.len())
}

fn byte_to_char(text: &str, at: usize) -> i32 {
    text[..at].chars().count() as i32
}

/// Character index of the next grapheme cluster boundary after `at`.
#[no_mangle]
pub extern "C" fn parley_grapheme_next(text: *const u8, len: i32, at: i32) -> i32 {
    if text.is_null() || len < 0 {
        return -1;
    }
    let bytes = unsafe { std::slice::from_raw_parts(text, len as usize) };
    let Ok(s) = std::str::from_utf8(bytes) else {
        return -2;
    };
    use unicode_segmentation::UnicodeSegmentation;
    let start = char_to_byte(s, at);
    let mut next = s.len();
    for (b, _) in s.grapheme_indices(true) {
        if b > start {
            next = b;
            break;
        }
    }
    byte_to_char(s, next)
}

/// Character index of the previous grapheme cluster boundary before `at`.
#[no_mangle]
pub extern "C" fn parley_grapheme_prev(text: *const u8, len: i32, at: i32) -> i32 {
    if text.is_null() || len < 0 {
        return -1;
    }
    let bytes = unsafe { std::slice::from_raw_parts(text, len as usize) };
    let Ok(s) = std::str::from_utf8(bytes) else {
        return -2;
    };
    use unicode_segmentation::UnicodeSegmentation;
    let start = char_to_byte(s, at);
    let mut prev = 0usize;
    for (b, _) in s.grapheme_indices(true) {
        if b < start {
            prev = b;
        } else {
            break;
        }
    }
    byte_to_char(s, prev)
}

#[no_mangle]
pub extern "C" fn parley_abi_version() -> i32 {
    3
}

/// Lay out UTF-8 text into lines and write one 24-byte record per line:
/// `start u32`, `end u32` (character offsets), `flags u32` (bit 0 means the
/// line is followed by an ellipsis), `width f32`, `height f32`, `baseline f32`.
///
/// `wrap != 0` breaks lines at `max_width`. `max_lines <= 0` keeps every line.
/// `ellipsis != 0` trims the last line that does not fit. The return value is
/// the number of lines, even when `cap` is too small, so the caller can grow
/// its buffer and retry.
#[no_mangle]
pub extern "C" fn parley_layout(
    text: *const u8,
    len: i32,
    size: f64,
    max_width: f64,
    max_lines: i32,
    wrap: i32,
    ellipsis: i32,
    out: *mut u8,
    cap: i32,
) -> i32 {
    if text.is_null() || len < 0 || size <= 0.0 || cap < 0 {
        return -1;
    }
    if out.is_null() && cap > 0 {
        return -1;
    }
    let bytes = unsafe { std::slice::from_raw_parts(text, len as usize) };
    let Ok(s) = std::str::from_utf8(bytes) else {
        return -2;
    };
    let limit = if wrap != 0 && max_width > 0.0 {
        Some(max_width as f32)
    } else {
        None
    };
    let layout = build_layout(s, size as f32, limit);

    let mut char_at = vec![0u32; s.len() + 1];
    let mut chars = 0u32;
    for (b, _) in s.char_indices() {
        char_at[b] = chars;
        chars += 1;
    }
    char_at[s.len()] = chars;

    let total = layout.lines().count();
    let keep = if max_lines > 0 && total > max_lines as usize {
        max_lines as usize
    } else {
        total
    };
    if cap < (keep * ROW_BYTES) as i32 {
        return keep as i32;
    }

    for (i, line) in layout.lines().take(keep).enumerate() {
        let metrics = line.metrics();
        let range = line.text_range();
        let mut end = range.end;
        let mut width = metrics.advance;
        let mut flags = 0u32;
        let clipped = total > keep || (max_width > 0.0 && width > max_width as f32);
        if ellipsis != 0 && i + 1 == keep && clipped {
            let (cut, w) = fit_ellipsis(s, range.start, range.end, size as f32, max_width as f32);
            end = cut;
            width = w;
            flags = 1;
        }
        let record = unsafe { out.add(i * ROW_BYTES) };
        unsafe {
            (record as *mut u32).write_unaligned(char_at[range.start]);
            (record.add(4) as *mut u32).write_unaligned(char_at[end]);
            (record.add(8) as *mut u32).write_unaligned(flags);
            (record.add(12) as *mut f32).write_unaligned(width);
            (record.add(16) as *mut f32).write_unaligned(metrics.line_height);
            (record.add(20) as *mut f32).write_unaligned(metrics.baseline);
        }
    }
    keep as i32
}

#[no_mangle]
pub extern "C" fn parley_noop(_p: *mut c_void) {}

#[no_mangle]
pub extern "C" fn parley_clusters(
    text: *const u8,
    len: i32,
    size: f64,
    out: *mut u8,
    cap: i32,
) -> i32 {
    if text.is_null() || out.is_null() || len <= 0 || size <= 0.0 || cap < 0 {
        return -1;
    }
    let bytes = unsafe { std::slice::from_raw_parts(text, len as usize) };
    let Ok(s) = std::str::from_utf8(bytes) else {
        return -2;
    };
    let layout = build_layout(s, size as f32, None);

    let mut i = 0i32;
    for line in layout.lines() {
        for item in line.items() {
            if let parley::PositionedLayoutItem::GlyphRun(g) = item {
                for cluster in g.run().clusters() {
                    let range = cluster.text_range();
                    if i < cap {
                        let at = unsafe { out.add((i as usize) * 16) };
                        unsafe {
                            (at as *mut u32).write_unaligned(range.start as u32);
                            (at.add(4) as *mut u32).write_unaligned(range.end as u32);
                            (at.add(8) as *mut f64).write_unaligned(cluster.advance() as f64);
                        }
                    }
                    i += 1;
                }
            }
        }
    }
    i
}
