use std::ffi::c_void;

#[no_mangle]
pub extern "C" fn parley_measure(text: *const u8, len: i32, size: f64) -> f64 {
    if text.is_null() || len <= 0 || size <= 0.0 {
        return -1.0;
    }
    let bytes = unsafe { std::slice::from_raw_parts(text, len as usize) };
    let Ok(s) = std::str::from_utf8(bytes) else {
        return -2.0;
    };
    let mut font_cx = parley::FontContext::new();
    let mut layout_cx: parley::LayoutContext<()> = parley::LayoutContext::new();
    let mut builder = layout_cx.ranged_builder(&mut font_cx, s, 1.0, true);
    builder.push_default(parley::StyleProperty::FontSize(size as f32));
    let mut layout = builder.build(s);
    layout.break_all_lines(None);
    layout.width() as f64
}

#[no_mangle]
pub extern "C" fn parley_abi_version() -> i32 {
    1
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
    let mut font_cx = parley::FontContext::new();
    let mut layout_cx: parley::LayoutContext<()> = parley::LayoutContext::new();
    let mut builder = layout_cx.ranged_builder(&mut font_cx, s, 1.0, true);
    builder.push_default(parley::StyleProperty::FontSize(size as f32));
    let mut layout = builder.build(s);
    layout.break_all_lines(None);

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
