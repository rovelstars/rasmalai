use std::cell::RefCell;
use std::ffi::c_void;

use parley::{FontStyle, FontWeight, GenericFamily, LineHeight, StyleProperty};

thread_local! {
    static FONTS: RefCell<Option<parley::FontContext>> = const { RefCell::new(None) };
}

#[derive(Clone, Copy)]
struct Style {
    weight: i32,
    italic: bool,
    line_height: f64,
    letter_spacing: f64,
}

impl Style {
    const DEFAULT: Self = Self {
        weight: 400,
        italic: false,
        line_height: 0.0,
        letter_spacing: 0.0,
    };

    fn new(weight: i32, italic: i32, line_height: f64, letter_spacing: f64) -> Self {
        Self {
            weight: normalize_weight(weight),
            italic: italic != 0,
            line_height: if line_height.is_finite() && line_height > 0.0 {
                line_height
            } else {
                0.0
            },
            letter_spacing: if letter_spacing.is_finite() {
                letter_spacing
            } else {
                0.0
            },
        }
    }
}

fn normalize_weight(weight: i32) -> i32 {
    if weight <= 0 {
        400
    } else {
        weight.clamp(100, 900)
    }
}

fn build_layout(text: &str, size: f32, max_width: Option<f32>, style: Style) -> parley::Layout<()> {
    FONTS.with(|slot| {
        let mut slot = slot.borrow_mut();
        let font_cx = slot.get_or_insert_with(parley::FontContext::new);
        let mut layout_cx: parley::LayoutContext<()> = parley::LayoutContext::new();
        let mut builder = layout_cx.ranged_builder(font_cx, text, 1.0, true);
        builder.push_default(StyleProperty::FontSize(size));
        if style.weight != Style::DEFAULT.weight {
            builder.push_default(StyleProperty::FontWeight(FontWeight::new(
                style.weight as f32,
            )));
        }
        if style.italic {
            builder.push_default(StyleProperty::FontStyle(FontStyle::Italic));
        }
        if style.letter_spacing != 0.0 {
            builder.push_default(StyleProperty::LetterSpacing(style.letter_spacing as f32));
        }
        if style.line_height > 0.0 {
            builder.push_default(StyleProperty::LineHeight(LineHeight::FontSizeRelative(
                style.line_height as f32,
            )));
        }
        let mut layout = builder.build(text);
        layout.break_all_lines(max_width);
        layout
    })
}

#[derive(Clone, Copy)]
struct Span {
    start: usize,
    end: usize,
    weight: i32,
    italic: bool,
    tracking: f32,
    family: GenericFamily,
    underline: bool,
    strikethrough: bool,
}

fn family_from_id(id: u32) -> GenericFamily {
    match id {
        1 => GenericFamily::Serif,
        2 => GenericFamily::Monospace,
        3 => GenericFamily::Cursive,
        4 => GenericFamily::Fantasy,
        5 => GenericFamily::SystemUi,
        6 => GenericFamily::UiSerif,
        7 => GenericFamily::UiSansSerif,
        8 => GenericFamily::UiMonospace,
        9 => GenericFamily::UiRounded,
        10 => GenericFamily::Emoji,
        11 => GenericFamily::Math,
        _ => GenericFamily::SansSerif,
    }
}

fn build_rich_layout(
    text: &str,
    size: f32,
    max_width: Option<f32>,
    line_height: f64,
    spans: &[Span],
) -> parley::Layout<()> {
    FONTS.with(|slot| {
        let mut slot = slot.borrow_mut();
        let font_cx = slot.get_or_insert_with(parley::FontContext::new);
        let mut layout_cx: parley::LayoutContext<()> = parley::LayoutContext::new();
        let mut builder = layout_cx.ranged_builder(font_cx, text, 1.0, true);
        builder.push_default(StyleProperty::FontSize(size));
        builder.push_default(StyleProperty::FontFamily(GenericFamily::SansSerif.into()));
        if line_height > 0.0 {
            builder.push_default(StyleProperty::LineHeight(LineHeight::FontSizeRelative(
                line_height as f32,
            )));
        }
        for span in spans {
            let range = span.start..span.end;
            builder.push(StyleProperty::FontFamily(span.family.into()), range.clone());
            builder.push(
                StyleProperty::FontWeight(FontWeight::new(span.weight as f32)),
                range.clone(),
            );
            builder.push(
                StyleProperty::FontStyle(if span.italic {
                    FontStyle::Italic
                } else {
                    FontStyle::Normal
                }),
                range.clone(),
            );
            builder.push(StyleProperty::LetterSpacing(span.tracking), range.clone());
            if span.underline {
                builder.push(StyleProperty::Underline(true), range.clone());
            }
            if span.strikethrough {
                builder.push(StyleProperty::Strikethrough(true), range);
            }
        }
        let mut layout = builder.build(text);
        layout.break_all_lines(max_width);
        layout
    })
}

fn text_width(text: &str, size: f32, style: Style) -> f32 {
    build_layout(text, size, None, style).width()
}

const ROW_BYTES: usize = 24;
const HEADER_BYTES: usize = 16;
const RUN_BYTES: usize = 32;
const SPAN_BYTES: usize = 32;
const _: () = assert!(SPAN_BYTES == 32);
const ELLIPSIS: &str = "…";

struct RichRow {
    start: u32,
    end: u32,
    flags: u32,
    width: f32,
    height: f32,
    baseline: f32,
}

struct RichRun {
    row: u32,
    start: u32,
    end: u32,
    flags: u32,
    x: f32,
    advance: f32,
    baseline: f32,
}

fn fit_ellipsis(
    text: &str,
    start: usize,
    end: usize,
    size: f32,
    max_width: f32,
    style: Style,
) -> (usize, f32) {
    let ellipsis_width = text_width(ELLIPSIS, size, style);
    if max_width <= 0.0 {
        return (
            end,
            text_width(&text[start..end], size, style) + ellipsis_width,
        );
    }
    if text_width(&text[start..end], size, style) + ellipsis_width <= max_width {
        return (
            end,
            text_width(&text[start..end], size, style) + ellipsis_width,
        );
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
        let w = text_width(&candidate, size, style);
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
    text_width(s, size as f32, Style::DEFAULT) as f64
}

/// Measure UTF-8 text with a styled face. `weight` is a CSS 100..900 value,
/// `italic` is a truthy flag, and `letter_spacing` is extra advance per
/// character in logical pixels.
#[no_mangle]
pub extern "C" fn parley_measure_styled(
    text: *const u8,
    len: i32,
    size: f64,
    weight: i32,
    italic: i32,
    letter_spacing: f64,
) -> f64 {
    if text.is_null() || len <= 0 || size <= 0.0 {
        return -1.0;
    }
    let bytes = unsafe { std::slice::from_raw_parts(text, len as usize) };
    let Ok(s) = std::str::from_utf8(bytes) else {
        return -2.0;
    };
    let style = Style::new(weight, italic, 0.0, letter_spacing);
    text_width(s, size as f32, style) as f64
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

fn span_char_to_byte(text: &str, at: u32) -> usize {
    if at > i32::MAX as u32 {
        return text.len();
    }
    char_to_byte(text, at as i32).min(text.len())
}

fn read_spans(text: &str, spans: *const u8, span_count: i32) -> Vec<Span> {
    let mut parsed = Vec::new();
    for i in 0..span_count as usize {
        let at = unsafe { spans.add(i * SPAN_BYTES) };
        let start_char = unsafe { (at as *const u32).read_unaligned() };
        let end_char = unsafe { (at.add(4) as *const u32).read_unaligned() };
        let weight = unsafe { (at.add(8) as *const i32).read_unaligned() };
        let italic = unsafe { (at.add(12) as *const u32).read_unaligned() };
        let tracking = unsafe { (at.add(16) as *const f64).read_unaligned() };
        let style = unsafe { (at.add(24) as *const u32).read_unaligned() };
        let start = span_char_to_byte(text, start_char);
        let end = span_char_to_byte(text, end_char);
        if end <= start {
            continue;
        }
        parsed.push(Span {
            start,
            end,
            weight: normalize_weight(weight),
            italic: italic != 0,
            tracking: if tracking.is_finite() {
                tracking as f32
            } else {
                0.0
            },
            family: family_from_id(style & 0xFF),
            underline: style & 0x100 != 0,
            strikethrough: style & 0x200 != 0,
        });
    }
    parsed
}

/// Character index of the next grapheme cluster boundary after `at`.
#[no_mangle]
pub extern "C" fn parley_grapheme_next(text: *const u8, len: i32, at: i32) -> i64 {
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
    byte_to_char(s, next) as i64
}

/// Character index of the previous grapheme cluster boundary before `at`.
#[no_mangle]
pub extern "C" fn parley_grapheme_prev(text: *const u8, len: i32, at: i32) -> i64 {
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
    byte_to_char(s, prev) as i64
}

#[no_mangle]
pub extern "C" fn parley_abi_version() -> i64 {
    6
}

#[allow(clippy::too_many_arguments)]
fn layout_run(
    text: *const u8,
    len: i32,
    size: f64,
    max_width: f64,
    max_lines: i32,
    wrap: i32,
    ellipsis: i32,
    style: Style,
    out: *mut u8,
    cap: i32,
) -> i64 {
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
    let layout = build_layout(s, size as f32, limit, style);

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
        return keep as i64;
    }

    for (i, line) in layout.lines().take(keep).enumerate() {
        let metrics = line.metrics();
        let range = line.text_range();
        let mut end = range.end;
        let mut width = metrics.advance;
        let mut flags = 0u32;
        let clipped = total > keep || (max_width > 0.0 && width > max_width as f32);
        if ellipsis != 0 && i + 1 == keep && clipped {
            let (cut, w) = fit_ellipsis(
                s,
                range.start,
                range.end,
                size as f32,
                max_width as f32,
                style,
            );
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
    keep as i64
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
) -> i64 {
    layout_run(
        text,
        len,
        size,
        max_width,
        max_lines,
        wrap,
        ellipsis,
        Style::DEFAULT,
        out,
        cap,
    )
}

/// `parley_layout` with a styled face. `weight` is a CSS 100..900 value,
/// `italic` is a truthy flag, `line_height` is `0.0` for the font's normal
/// spacing or a multiplier of `size`, and `letter_spacing` is extra advance
/// per character in logical pixels.
#[no_mangle]
pub extern "C" fn parley_layout_styled(
    text: *const u8,
    len: i32,
    size: f64,
    max_width: f64,
    max_lines: i32,
    wrap: i32,
    ellipsis: i32,
    weight: i32,
    italic: i32,
    line_height: f64,
    letter_spacing: f64,
    out: *mut u8,
    cap: i32,
) -> i64 {
    layout_run(
        text,
        len,
        size,
        max_width,
        max_lines,
        wrap,
        ellipsis,
        Style::new(weight, italic, line_height, letter_spacing),
        out,
        cap,
    )
}

#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub extern "C" fn parley_layout_rich(
    text: *const u8,
    len: i32,
    size: f64,
    max_width: f64,
    max_lines: i32,
    wrap: i32,
    ellipsis: i32,
    line_height: f64,
    spans: *const u8,
    span_count: i32,
    out: *mut u8,
    cap: i32,
) -> i64 {
    if text.is_null() || len < 0 || !size.is_finite() || size <= 0.0 || cap < 0 {
        return -1;
    }
    if span_count < 0 || (span_count > 0 && spans.is_null()) {
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
    let line_height = if line_height.is_finite() && line_height > 0.0 {
        line_height
    } else {
        0.0
    };
    let parsed = read_spans(s, spans, span_count);
    let layout = build_rich_layout(s, size as f32, limit, line_height, &parsed);

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
    let ellipsis_width = if ellipsis != 0 {
        text_width(ELLIPSIS, size as f32, Style::DEFAULT)
    } else {
        0.0
    };

    let mut rows = Vec::with_capacity(keep);
    let mut runs = Vec::new();
    for (i, line) in layout.lines().take(keep).enumerate() {
        let metrics = line.metrics();
        let range = line.text_range();
        let row_start = char_at[range.start];
        let row_end = char_at[range.end];
        let clipped = total > keep || (max_width > 0.0 && metrics.advance > max_width as f32);
        let show_ellipsis = ellipsis != 0 && i + 1 == keep && clipped;
        rows.push(RichRow {
            start: row_start,
            end: row_end,
            flags: if show_ellipsis { 1 } else { 0 },
            width: metrics.advance,
            height: metrics.line_height,
            baseline: metrics.baseline,
        });

        let mut after_last_run = 0.0f32;
        let mut has_run = false;
        let line_runs: Vec<_> = line.runs().collect();
        let mut run_starts = vec![0.0f32; line_runs.len()];
        let mut line_x = 0.0f32;
        for r in &line_runs {
            if r.index() < run_starts.len() {
                run_starts[r.index()] = line_x;
            }
            line_x += r.advance();
        }
        for item in line.items() {
            if let parley::PositionedLayoutItem::GlyphRun(g) = item {
                let run_index = g.run().index();
                let run_start = run_starts.get(run_index).copied().unwrap_or(0.0);
                let rel0 = g.offset() - run_start;
                let rel1 = rel0 + g.advance();
                let mut cluster_start: Option<usize> = None;
                let mut cluster_end: Option<usize> = None;
                let mut cluster_x = 0.0f32;
                for cluster in g.run().visual_clusters() {
                    let chunk = cluster.advance();
                    let c0 = cluster_x;
                    let c1 = cluster_x + chunk;
                    if c1 > rel0 + 0.01 && c0 < rel1 - 0.01 {
                        let cr = cluster.text_range();
                        cluster_start = Some(cluster_start.map_or(cr.start, |v| v.min(cr.start)));
                        cluster_end = Some(cluster_end.map_or(cr.end, |v| v.max(cr.end)));
                    }
                    cluster_x += chunk;
                }
                let (range_start, range_end) = match (cluster_start, cluster_end) {
                    (Some(a), Some(b)) => (a, b),
                    _ => {
                        let r = g.run().text_range();
                        (r.start, r.end)
                    }
                };
                let start = byte_to_char(s, range_start).clamp(row_start as i32, row_end as i32);
                let end = byte_to_char(s, range_end).clamp(row_start as i32, row_end as i32);
                let end = end.max(start);
                let x = g.offset();
                let advance = g.advance();
                runs.push(RichRun {
                    row: i as u32,
                    start: start as u32,
                    end: end as u32,
                    flags: 0,
                    x,
                    advance,
                    baseline: g.baseline(),
                });
                after_last_run = x + advance;
                has_run = true;
            }
        }
        if show_ellipsis {
            runs.push(RichRun {
                row: i as u32,
                start: row_end,
                end: row_end,
                flags: 1,
                x: if has_run { after_last_run } else { 0.0 },
                advance: ellipsis_width,
                baseline: metrics.baseline,
            });
        }
    }

    let required = HEADER_BYTES + rows.len() * ROW_BYTES + runs.len() * RUN_BYTES;
    if out.is_null() || (cap as usize) < required {
        return required as i64;
    }

    unsafe {
        (out as *mut u32).write_unaligned(rows.len() as u32);
        (out.add(4) as *mut u32).write_unaligned(runs.len() as u32);
        (out.add(8) as *mut u64).write_unaligned(0);
    }
    for (i, row) in rows.iter().enumerate() {
        let at = unsafe { out.add(HEADER_BYTES + i * ROW_BYTES) };
        unsafe {
            (at as *mut u32).write_unaligned(row.start);
            (at.add(4) as *mut u32).write_unaligned(row.end);
            (at.add(8) as *mut u32).write_unaligned(row.flags);
            (at.add(12) as *mut f32).write_unaligned(row.width);
            (at.add(16) as *mut f32).write_unaligned(row.height);
            (at.add(20) as *mut f32).write_unaligned(row.baseline);
        }
    }
    let run_base = HEADER_BYTES + rows.len() * ROW_BYTES;
    for (i, run) in runs.iter().enumerate() {
        let at = unsafe { out.add(run_base + i * RUN_BYTES) };
        unsafe {
            (at as *mut u32).write_unaligned(run.row);
            (at.add(4) as *mut u32).write_unaligned(run.start);
            (at.add(8) as *mut u32).write_unaligned(run.end);
            (at.add(12) as *mut u32).write_unaligned(run.flags);
            (at.add(16) as *mut f32).write_unaligned(run.x);
            (at.add(20) as *mut f32).write_unaligned(run.advance);
            (at.add(24) as *mut f32).write_unaligned(run.baseline);
            (at.add(28) as *mut f32).write_unaligned(0.0);
        }
    }
    required as i64
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
) -> i64 {
    if text.is_null() || out.is_null() || len <= 0 || size <= 0.0 || cap < 0 {
        return -1;
    }
    let bytes = unsafe { std::slice::from_raw_parts(text, len as usize) };
    let Ok(s) = std::str::from_utf8(bytes) else {
        return -2;
    };
    let layout = build_layout(s, size as f32, None, Style::DEFAULT);

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
    i as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "Styled shaping probe";

    fn measure(text: &str, size: f64, weight: i32, italic: i32, tracking: f64) -> f64 {
        parley_measure_styled(
            text.as_ptr(),
            text.len() as i32,
            size,
            weight,
            italic,
            tracking,
        )
    }

    fn layout_rows(
        text: &str,
        size: f64,
        max_width: f64,
        max_lines: i32,
        weight: i32,
        italic: i32,
        line_height: f64,
        tracking: f64,
    ) -> Vec<(f32, f32)> {
        let mut out = vec![0u8; ROW_BYTES * 16];
        let rows = parley_layout_styled(
            text.as_ptr(),
            text.len() as i32,
            size,
            max_width,
            max_lines,
            1,
            0,
            weight,
            italic,
            line_height,
            tracking,
            out.as_mut_ptr(),
            out.len() as i32,
        );
        assert!(rows >= 0, "layout failed: {rows}");
        (0..rows as usize)
            .map(|i| {
                let at = i * ROW_BYTES;
                let width = f32::from_le_bytes(out[at + 12..at + 16].try_into().unwrap());
                let height = f32::from_le_bytes(out[at + 16..at + 20].try_into().unwrap());
                (width, height)
            })
            .collect()
    }

    #[test]
    fn abi_version_is_six() {
        assert_eq!(parley_abi_version(), 6);
    }

    #[test]
    fn bold_is_wider_than_regular() {
        let regular = measure(TEXT, 20.0, 400, 0, 0.0);
        let bold = measure(TEXT, 20.0, 700, 0, 0.0);
        assert!(regular > 0.0, "regular width: {regular}");
        assert!(
            bold > regular,
            "bold {bold} not wider than regular {regular}"
        );
    }

    #[test]
    fn italic_shapes_without_failing() {
        let upright = measure(TEXT, 20.0, 400, 0, 0.0);
        let italic = measure(TEXT, 20.0, 400, 1, 0.0);
        assert!(upright > 0.0, "upright width: {upright}");
        assert!(italic > 0.0 && italic.is_finite(), "italic width: {italic}");
        let rows = layout_rows(TEXT, 20.0, 0.0, 0, 400, 1, 0.0, 0.0);
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn tracking_is_wider_than_plain() {
        let plain = measure(TEXT, 20.0, 400, 0, 0.0);
        let tracked = measure(TEXT, 20.0, 400, 0, 2.0);
        assert!(plain > 0.0, "plain width: {plain}");
        assert!(
            tracked > plain,
            "tracked {tracked} not wider than plain {plain}"
        );
    }

    #[test]
    fn line_height_multiplier_scales_rows() {
        let size = 20.0;
        let normal = layout_rows(TEXT, size, 70.0, 0, 400, 0, 0.0, 0.0);
        let doubled = layout_rows(TEXT, size, 70.0, 0, 400, 0, 2.0, 0.0);
        assert!(normal.len() >= 2, "expected wrapped rows: {normal:?}");
        assert_eq!(normal.len(), doubled.len());
        let normal_height = normal[0].1;
        let doubled_height = doubled[0].1;
        assert!(normal_height > 0.0, "normal height: {normal_height}");
        let expected = size as f32 * 2.0;
        assert!(
            (doubled_height - expected).abs() < 0.5,
            "line height {doubled_height} != {expected}"
        );
        assert!(doubled_height > normal_height);
    }

    #[test]
    fn default_layout_matches_styled_default() {
        let mut plain = vec![0u8; ROW_BYTES * 8];
        let plain_rows = parley_layout(
            TEXT.as_ptr(),
            TEXT.len() as i32,
            20.0,
            0.0,
            0,
            0,
            0,
            plain.as_mut_ptr(),
            plain.len() as i32,
        );
        let mut styled = vec![0u8; ROW_BYTES * 8];
        let styled_rows = parley_layout_styled(
            TEXT.as_ptr(),
            TEXT.len() as i32,
            20.0,
            0.0,
            0,
            0,
            0,
            400,
            0,
            0.0,
            0.0,
            styled.as_mut_ptr(),
            styled.len() as i32,
        );
        assert_eq!(plain_rows, styled_rows);
        assert_eq!(plain, styled);
    }

    struct OutRow {
        start: u32,
        end: u32,
        flags: u32,
        width: f32,
        height: f32,
        baseline: f32,
    }

    struct OutRun {
        row: u32,
        start: u32,
        end: u32,
        flags: u32,
        x: f32,
        advance: f32,
        baseline: f32,
        reserved: f32,
    }

    fn make_span(
        start: u32,
        end: u32,
        weight: i32,
        italic: u32,
        tracking: f64,
    ) -> [u8; SPAN_BYTES] {
        make_span_styled(start, end, weight, italic, tracking, 0)
    }

    fn make_span_styled(
        start: u32,
        end: u32,
        weight: i32,
        italic: u32,
        tracking: f64,
        style: u32,
    ) -> [u8; SPAN_BYTES] {
        let mut span = [0u8; SPAN_BYTES];
        span[0..4].copy_from_slice(&start.to_le_bytes());
        span[4..8].copy_from_slice(&end.to_le_bytes());
        span[8..12].copy_from_slice(&weight.to_le_bytes());
        span[12..16].copy_from_slice(&italic.to_le_bytes());
        span[16..24].copy_from_slice(&tracking.to_le_bytes());
        span[24..28].copy_from_slice(&style.to_le_bytes());
        span
    }

    fn family_style(family: u32) -> u32 {
        family & 0xFF
    }

    fn decoration_style(family: u32, underline: bool, strikethrough: bool) -> u32 {
        family_style(family)
            | if underline { 1 << 8 } else { 0 }
            | if strikethrough { 1 << 9 } else { 0 }
    }

    fn parse_rich(raw: &[u8]) -> (Vec<OutRow>, Vec<OutRun>) {
        let row_count = u32::from_le_bytes(raw[0..4].try_into().unwrap()) as usize;
        let run_count = u32::from_le_bytes(raw[4..8].try_into().unwrap()) as usize;
        let mut rows = Vec::with_capacity(row_count);
        for i in 0..row_count {
            let at = HEADER_BYTES + i * ROW_BYTES;
            rows.push(OutRow {
                start: u32::from_le_bytes(raw[at..at + 4].try_into().unwrap()),
                end: u32::from_le_bytes(raw[at + 4..at + 8].try_into().unwrap()),
                flags: u32::from_le_bytes(raw[at + 8..at + 12].try_into().unwrap()),
                width: f32::from_le_bytes(raw[at + 12..at + 16].try_into().unwrap()),
                height: f32::from_le_bytes(raw[at + 16..at + 20].try_into().unwrap()),
                baseline: f32::from_le_bytes(raw[at + 20..at + 24].try_into().unwrap()),
            });
        }
        let base = HEADER_BYTES + row_count * ROW_BYTES;
        let mut runs = Vec::with_capacity(run_count);
        for i in 0..run_count {
            let at = base + i * RUN_BYTES;
            runs.push(OutRun {
                row: u32::from_le_bytes(raw[at..at + 4].try_into().unwrap()),
                start: u32::from_le_bytes(raw[at + 4..at + 8].try_into().unwrap()),
                end: u32::from_le_bytes(raw[at + 8..at + 12].try_into().unwrap()),
                flags: u32::from_le_bytes(raw[at + 12..at + 16].try_into().unwrap()),
                x: f32::from_le_bytes(raw[at + 16..at + 20].try_into().unwrap()),
                advance: f32::from_le_bytes(raw[at + 20..at + 24].try_into().unwrap()),
                baseline: f32::from_le_bytes(raw[at + 24..at + 28].try_into().unwrap()),
                reserved: f32::from_le_bytes(raw[at + 28..at + 32].try_into().unwrap()),
            });
        }
        (rows, runs)
    }

    #[allow(clippy::too_many_arguments)]
    fn rich(
        text: &str,
        size: f64,
        max_width: f64,
        max_lines: i32,
        wrap: i32,
        ellipsis: i32,
        line_height: f64,
        spans: &[[u8; SPAN_BYTES]],
    ) -> (Vec<OutRow>, Vec<OutRun>, Vec<u8>) {
        let span_ptr = if spans.is_empty() {
            std::ptr::null()
        } else {
            spans.as_ptr() as *const u8
        };
        let required = parley_layout_rich(
            text.as_ptr(),
            text.len() as i32,
            size,
            max_width,
            max_lines,
            wrap,
            ellipsis,
            line_height,
            span_ptr,
            spans.len() as i32,
            std::ptr::null_mut(),
            0,
        );
        assert!(required >= HEADER_BYTES as i64, "required: {required}");
        let mut out = vec![0u8; required as usize];
        let ret = parley_layout_rich(
            text.as_ptr(),
            text.len() as i32,
            size,
            max_width,
            max_lines,
            wrap,
            ellipsis,
            line_height,
            span_ptr,
            spans.len() as i32,
            out.as_mut_ptr(),
            out.len() as i32,
        );
        assert_eq!(ret, required);
        let (rows, runs) = parse_rich(&out);
        (rows, runs, out)
    }

    #[test]
    fn rich_bold_span_splits_runs_and_widens() {
        let text = "Bold span probe";
        let (plain_rows, plain_runs, _) = rich(text, 20.0, 0.0, 0, 1, 0, 0.0, &[]);
        let bold = make_span(0, 4, 700, 0, 0.0);
        let (rows, runs, _) = rich(text, 20.0, 0.0, 0, 1, 0, 0.0, &[bold]);
        assert_eq!(plain_rows.len(), 1);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].width > 0.0);
        assert!(rows[0].height > 0.0);
        assert!(rows[0].baseline > 0.0);
        assert!(runs.len() >= 2, "expected split runs, got {}", runs.len());
        let bold_run = runs
            .iter()
            .find(|run| run.start == 0 && run.end == 4)
            .expect("bold run on chars 0..4");
        assert_eq!(bold_run.row, 0);
        assert!(bold_run.baseline.is_finite());
        let plain_width: f32 = plain_runs.iter().map(|run| run.advance).sum();
        let rich_width: f32 = runs.iter().map(|run| run.advance).sum();
        assert!(
            rich_width > plain_width,
            "bold width {rich_width} not wider than plain {plain_width}"
        );
    }

    #[test]
    fn rich_italic_and_tracking_spans_split_runs() {
        let text = "abcde fghij klmno";
        let italic = make_span(0, 5, 400, 1, 0.0);
        let tracked = make_span(12, 17, 400, 0, 2.0);
        let (rows, runs, _) = rich(text, 20.0, 0.0, 0, 1, 0, 0.0, &[italic, tracked]);
        assert_eq!(rows.len(), 1);
        assert!(runs.len() >= 3, "expected three runs, got {}", runs.len());
        assert!(
            runs.iter().any(|run| run.start == 0 && run.end == 5),
            "missing italic run: {runs:?}",
            runs = runs.iter().map(|r| (r.start, r.end)).collect::<Vec<_>>()
        );
        assert!(
            runs.iter().any(|run| run.start == 12 && run.end == 17),
            "missing tracked run"
        );
        assert!(runs.iter().all(|run| run.row == 0));
    }

    #[test]
    fn rich_runs_belong_to_wrapped_rows() {
        let text = "Wrap these words across several rows please";
        let (rows, runs, _) = rich(text, 16.0, 70.0, 0, 1, 0, 0.0, &[]);
        assert!(rows.len() >= 2, "expected wrapped rows, got {}", rows.len());
        let last_char = text.chars().count() as u32;
        assert_eq!(rows[0].start, 0);
        assert_eq!(rows[rows.len() - 1].end, last_char);
        for i in 1..rows.len() {
            assert!(
                rows[i].start >= rows[i - 1].start,
                "row starts not ordered"
            );
        }
        for i in 0..rows.len() {
            assert!(
                runs.iter().any(|run| run.row == i as u32),
                "row {i} has no runs"
            );
        }
        for run in &runs {
            let row = &rows[run.row as usize];
            assert!(run.row < rows.len() as u32);
            assert!(run.start >= row.start, "run before row start");
            assert!(run.end <= row.end, "run after row end");
            assert!(run.start <= run.end);
            assert!(run.baseline.is_finite());
            assert!(run.reserved == 0.0);
        }
    }

    #[test]
    fn rich_zero_spans_match_styled_regular_rows() {
        let text = "Regular rich layout";
        let (rows, runs, raw) = rich(text, 20.0, 90.0, 0, 1, 0, 0.0, &[]);
        assert!(!rows.is_empty());
        assert!(!runs.is_empty());
        let mut styled = vec![0u8; rows.len() * ROW_BYTES];
        let styled_rows = parley_layout_styled(
            text.as_ptr(),
            text.len() as i32,
            20.0,
            90.0,
            0,
            1,
            0,
            400,
            0,
            0.0,
            0.0,
            styled.as_mut_ptr(),
            styled.len() as i32,
        );
        assert_eq!(styled_rows as usize, rows.len());
        assert_eq!(
            &raw[HEADER_BYTES..HEADER_BYTES + rows.len() * ROW_BYTES],
            &styled[..]
        );
    }

    #[test]
    fn rich_span_offsets_are_characters_not_bytes() {
        let text = "aé中b";
        let italic = make_span(1, 2, 400, 1, 0.0);
        let bold = make_span(2, 4, 700, 0, 1.0);
        let (rows, runs, _) = rich(text, 20.0, 0.0, 0, 1, 0, 0.0, &[italic, bold]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].start, 0);
        assert_eq!(rows[0].end, 4);
        assert!(!runs.is_empty());
        for run in &runs {
            assert_eq!(run.row, 0);
            assert!(run.start <= run.end);
            assert!(run.end <= 4);
        }
        let clamped = make_span(1, 999, 400, 0, 0.0);
        let (rows, _, _) = rich(text, 20.0, 0.0, 0, 1, 0, 0.0, &[clamped]);
        assert_eq!(rows[0].end, 4);
    }

    #[test]
    fn rich_small_cap_returns_required_then_retry_succeeds() {
        let text = "Cap retry probe";
        let bold = make_span(0, 3, 700, 0, 0.0);
        let spans = [bold];
        let span_ptr = spans.as_ptr() as *const u8;
        let required = parley_layout_rich(
            text.as_ptr(),
            text.len() as i32,
            20.0,
            0.0,
            0,
            1,
            0,
            0.0,
            span_ptr,
            spans.len() as i32,
            std::ptr::null_mut(),
            0,
        );
        assert!(required > HEADER_BYTES as i64, "required: {required}");
        let mut small = vec![0xAAu8; required as usize - 1];
        let ret = parley_layout_rich(
            text.as_ptr(),
            text.len() as i32,
            20.0,
            0.0,
            0,
            1,
            0,
            0.0,
            span_ptr,
            spans.len() as i32,
            small.as_mut_ptr(),
            small.len() as i32,
        );
        assert_eq!(ret, required);
        assert!(
            small.iter().all(|&byte| byte == 0xAA),
            "small cap wrote into the buffer"
        );
        let mut out = vec![0u8; required as usize];
        let retry = parley_layout_rich(
            text.as_ptr(),
            text.len() as i32,
            20.0,
            0.0,
            0,
            1,
            0,
            0.0,
            span_ptr,
            spans.len() as i32,
            out.as_mut_ptr(),
            out.len() as i32,
        );
        assert_eq!(retry, required);
        let (rows, runs) = parse_rich(&out);
        assert!(!rows.is_empty());
        assert!(!runs.is_empty());
    }

    #[test]
    fn rich_ellipsis_flag_and_run_when_max_lines_clips() {
        let text = "The quick brown fox jumps over the lazy dog";
        let (rows, runs, _) = rich(text, 20.0, 70.0, 1, 1, 1, 0.0, &[]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].flags & 1, 1, "missing ellipsis flag");
        let last = runs.last().expect("ellipsis run");
        assert_eq!(last.flags, 1);
        assert_eq!(last.row, 0);
        assert_eq!(last.start, rows[0].end);
        assert_eq!(last.end, rows[0].end);
        assert!(last.advance > 0.0, "ellipsis advance: {}", last.advance);
        if runs.len() > 1 {
            let previous = &runs[runs.len() - 2];
            assert!(
                last.x >= previous.x + previous.advance - 0.01,
                "ellipsis not after the last run"
            );
        }
    }

    #[test]
    fn rich_span_word_parses_family_and_decoration_bits() {
        let text = "abcdefgh";
        assert_eq!(SPAN_BYTES, 32);
        let mut span = make_span_styled(0, 4, 400, 0, 0.0, decoration_style(2, true, true));
        span[28..32].copy_from_slice(&u32::MAX.to_le_bytes());
        let parsed = read_spans(text, span.as_ptr(), 1);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].start, 0);
        assert_eq!(parsed[0].end, 4);
        assert_eq!(parsed[0].family, GenericFamily::Monospace);
        assert!(parsed[0].underline);
        assert!(parsed[0].strikethrough);
        assert_eq!(parsed[0].weight, 400);
        assert!(!parsed[0].italic);
        assert_eq!(parsed[0].tracking, 0.0);

        let plain = make_span(0, 1, 400, 0, 0.0);
        let parsed = read_spans(text, plain.as_ptr(), 1);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].family, GenericFamily::SansSerif);
        assert!(!parsed[0].underline);
        assert!(!parsed[0].strikethrough);
    }

    #[test]
    fn rich_serif_and_monospace_spans_shape_expected_runs() {
        let text = "iiiiWWWW";
        let serif = make_span_styled(0, 4, 400, 0, 0.0, family_style(1));
        let mono = make_span_styled(4, 8, 400, 0, 0.0, family_style(2));
        let (rows, runs, _) = rich(text, 24.0, 0.0, 0, 1, 0, 0.0, &[serif, mono]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].start, 0);
        assert_eq!(rows[0].end, 8);
        assert!(rows[0].width > 0.0);
        assert!(
            runs.iter().any(|run| run.start == 0 && run.end == 4),
            "missing serif run: {:?}",
            runs.iter()
                .map(|run| (run.start, run.end))
                .collect::<Vec<_>>()
        );
        assert!(
            runs.iter().any(|run| run.start == 4 && run.end == 8),
            "missing monospace run: {:?}",
            runs.iter()
                .map(|run| (run.start, run.end))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn rich_monospace_advance_differs_from_sans() {
        let text = "iiiWWWMMM";
        let sans = make_span_styled(0, 9, 400, 0, 0.0, family_style(0));
        let mono = make_span_styled(0, 9, 400, 0, 0.0, family_style(2));
        let (sans_rows, _, _) = rich(text, 24.0, 0.0, 0, 1, 0, 0.0, &[sans]);
        let (mono_rows, _, _) = rich(text, 24.0, 0.0, 0, 1, 0, 0.0, &[mono]);
        assert_eq!(sans_rows.len(), 1);
        assert_eq!(mono_rows.len(), 1);
        assert!(
            sans_rows[0].width > 0.0,
            "sans width: {}",
            sans_rows[0].width
        );
        assert!(
            mono_rows[0].width > 0.0,
            "mono width: {}",
            mono_rows[0].width
        );
        let delta = (mono_rows[0].width - sans_rows[0].width).abs();
        assert!(
            delta > 0.5,
            "monospace width {} matches sans width {}",
            mono_rows[0].width,
            sans_rows[0].width
        );
    }

    #[test]
    fn rich_underline_and_strikethrough_spans_report_decoration() {
        let text = "under strike";
        let underline = make_span_styled(0, 5, 400, 0, 0.0, decoration_style(0, true, false));
        let strike = make_span_styled(6, 12, 400, 0, 0.0, decoration_style(0, false, true));
        let spans = [underline, strike];
        let (rows, runs, _) = rich(text, 20.0, 0.0, 0, 1, 0, 0.0, &spans);
        assert_eq!(rows.len(), 1);
        assert!(!runs.is_empty());

        let parsed = read_spans(text, spans.as_ptr() as *const u8, spans.len() as i32);
        let layout = build_rich_layout(text, 20.0, None, 0.0, &parsed);
        let mut saw_underline = false;
        let mut saw_strikethrough = false;
        for line in layout.lines() {
            for item in line.items() {
                if let parley::PositionedLayoutItem::GlyphRun(g) = item {
                    let style = g.style();
                    let range = g.run().text_range();
                    if range.start < 5 && range.end > 0 && style.underline.is_some() {
                        saw_underline = true;
                    }
                    if range.start < 12 && range.end > 6 && style.strikethrough.is_some() {
                        saw_strikethrough = true;
                    }
                }
            }
        }
        assert!(saw_underline, "underline decoration not reported");
        assert!(saw_strikethrough, "strikethrough decoration not reported");
    }

    #[test]
    fn rich_unknown_family_falls_back_without_panic() {
        let text = "Fallback probe";
        let unknown = make_span_styled(0, 8, 400, 0, 0.0, family_style(200));
        let sans = make_span_styled(0, 8, 400, 0, 0.0, family_style(0));
        let (rows, runs, raw) = rich(text, 20.0, 0.0, 0, 1, 0, 0.0, &[unknown]);
        assert_eq!(rows.len(), 1);
        assert!(!runs.is_empty());
        let (_, _, sans_raw) = rich(text, 20.0, 0.0, 0, 1, 0, 0.0, &[sans]);
        assert_eq!(raw, sans_raw, "unknown family changed the layout");
    }

}
