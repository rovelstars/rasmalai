use super::common::*;
use super::collections::*;
#[cfg(test)]
use super::io::*;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_string_from_cstr(s: *const u8) -> *mut u8 {
    if s.is_null() {
        return str_alloc(0);
    }
    let mut len = 0usize;
    unsafe {
        while *s.add(len) != 0 {
            len += 1;
        }
        let out = str_alloc(len);
        if out.is_null() {
            return out;
        }
        std::ptr::copy_nonoverlapping(s, out.add(STR_HEADER), len);
        out.add(STR_HEADER + len).write(0);
        out
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_string_concat(a: *const u8, b: *const u8) -> *mut u8 {
    unsafe {
        let (la, lb) = (str_len(a), str_len(b));
        let out = str_alloc(la.wrapping_add(lb));
        if out.is_null() {
            return out;
        }
        let dst = out.add(STR_HEADER);
        if la > 0 {
            std::ptr::copy_nonoverlapping(a.add(STR_HEADER), dst, la);
        }
        if lb > 0 {
            std::ptr::copy_nonoverlapping(b.add(STR_HEADER), dst.add(la), lb);
        }
        dst.add(la.wrapping_add(lb)).write(0);
        out
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_string_eq(a: *const u8, b: *const u8) -> bool {
    if a == b {
        return true;
    }
    unsafe {
        if a.is_null() || b.is_null() {
            return false;
        }
        let (la, lb) = (str_len(a), str_len(b));
        if la != lb {
            return false;
        }
        if la == 0 {
            return true;
        }
        libc_memcmp(a.add(STR_HEADER), b.add(STR_HEADER), la) == 0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_string_cmp(a: *const u8, b: *const u8) -> i64 {
    if a == b {
        return 0;
    }
    unsafe {
        if a.is_null() {
            return -1;
        }
        if b.is_null() {
            return 1;
        }
        let (la, lb) = (str_len(a), str_len(b));
        let prefix = libc_memcmp(a.add(STR_HEADER), b.add(STR_HEADER), la.min(lb));
        if prefix != 0 {
            return prefix as i64;
        }
        if la == lb {
            return 0;
        }
        if la < lb {
            return -1;
        }
        1
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_release_str(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        let len = str_len(ptr);
        rnx_release(ptr, str_body_size(len), None);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_float_to_str(bits: u64) -> *mut u8 {
    let text = fmt_float(f64::from_bits(bits));
    let bytes = text.as_bytes();
    let out = str_alloc(bytes.len());
    if out.is_null() {
        return out;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), out.add(STR_HEADER), bytes.len());
        out.add(STR_HEADER).add(bytes.len()).write(0);
        out
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_bool_to_str(val: i64) -> *mut u8 {
    if val != 0 {
        alloc_str("true")
    } else {
        alloc_str("false")
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_any_to_str(any: u64) -> *mut u8 {
    unsafe {
        let payload = rnx_any_unbox(any);
        match rnx_any_tag(any) {
            t if t == TAG_INT as u64 => rnx_int_to_str(payload as i64),
            t if t == TAG_BOOL as u64 => rnx_bool_to_str(payload as i64),
            t if t == TAG_FLOAT as u64 => rnx_float_to_str(payload),
            t if t == TAG_STR as u64 => {
                if payload == 0 {
                    alloc_str("null")
                } else {
                    rnx_retain(payload as *mut u8);
                    payload as *mut u8
                }
            }
            _ => alloc_str(&super::pretty::pretty_any_plain(any)),
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_eq_any_str(any: u64, s: *const u8) -> u64 {
    unsafe {
        if rnx_any_tag(any) != TAG_STR as u64 {
            return 0;
        }
        let payload = rnx_any_unbox(any) as *const u8;
        rnx_string_eq(payload, s) as u64
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_eq_any(a: u64, b: u64) -> u64 {
    unsafe {
        let (ta, tb) = (rnx_any_tag(a), rnx_any_tag(b));
        if ta != tb {
            return 0;
        }
        let (pa, pb) = (rnx_any_unbox(a), rnx_any_unbox(b));
        match ta as u32 {
            TAG_INT => (pa as i64 == pb as i64) as u64,
            TAG_BOOL => ((pa != 0) == (pb != 0)) as u64,
            TAG_FLOAT => (f64::from_bits(pa) == f64::from_bits(pb)) as u64,
            TAG_STR => rnx_string_eq(pa as *const u8, pb as *const u8) as u64,
            _ => (pa == pb) as u64,
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_int_to_str(val: i64) -> *mut u8 {    let text = val.to_string();
    let bytes = text.as_bytes();
    let out = str_alloc(bytes.len());
    if out.is_null() {
        return out;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), out.add(STR_HEADER), bytes.len());
        out.add(STR_HEADER).add(bytes.len()).write(0);
        out
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_string_len(s: *const u8) -> i64 {
    str_chars(s) as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_string_slice(s: *const u8, start: i64, end: i64) -> *mut u8 {
    let bytes = str_bytes(s);
    let ascii = str_ascii(s);
    let total = if ascii { bytes.len() as i64 } else { str_chars(s) as i64 };
    let lo = start.clamp(0, total) as usize;
    let hi = end.clamp(0, total) as usize;
    if lo >= hi {
        return alloc_str("");
    }
    if ascii {
        let text = unsafe { std::str::from_utf8_unchecked(&bytes[lo..hi]) };
        return alloc_str(text);
    }
    let text = unsafe { std::str::from_utf8_unchecked(bytes) };
    let mut start_b = text.len();
    let mut end_b = text.len();
    let mut ci = 0usize;
    for (b, _) in text.char_indices() {
        if ci == lo {
            start_b = b;
        }
        if ci == hi {
            end_b = b;
            break;
        }
        ci += 1;
    }
    if ci < hi {
        end_b = text.len();
    }
    if start_b > end_b {
        start_b = end_b;
    }
    alloc_str(&text[start_b..end_b])
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_string_index_of(s: *const u8, needle: *const u8) -> i64 {
    let hay = str_bytes(s);
    let nd = str_bytes(needle);
    if nd.is_empty() {
        return 0;
    }
    let text = unsafe { std::str::from_utf8_unchecked(hay) };
    let pat = std::str::from_utf8(nd).unwrap_or("");
    match text.find(pat) {
        Some(byte) => {
            if str_ascii(s) {
                byte as i64
            } else {
                text[..byte].chars().count() as i64
            }
        }
        None => -1,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_string_index_of_from(s: *const u8, needle: *const u8, from: i64) -> i64 {
    let hay = str_bytes(s);
    let ascii = str_ascii(s);
    let total = if ascii { hay.len() as i64 } else { str_chars(s) as i64 };
    let start = from.clamp(0, total);
    let nd = str_bytes(needle);
    if nd.is_empty() {
        return start;
    }
    let text = unsafe { std::str::from_utf8_unchecked(hay) };
    let pat = std::str::from_utf8(nd).unwrap_or("");
    if ascii {
        let su = start as usize;
        match text[su..].find(pat) {
            Some(rel) => start + rel as i64,
            None => -1,
        }
    } else {
        let mut byte_off = text.len();
        let mut ci = 0usize;
        for (b, _) in text.char_indices() {
            if ci == start as usize {
                byte_off = b;
                break;
            }
            ci += 1;
        }
        match text[byte_off..].find(pat) {
            Some(rel) => start + text[byte_off..byte_off + rel].chars().count() as i64,
            None => -1,
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_string_split(s: *const u8, needle: *const u8) -> *mut u8 {
    unsafe {
        let text = String::from_utf8_lossy(str_bytes(s));
        let nd = String::from_utf8_lossy(str_bytes(needle));
        let out = rnx_array_new(0, 8);
        if out.is_null() {
            return out;
        }
        let nd_s: &str = &nd;
        if nd_s.is_empty() {
            let mut buf = [0u8; 4];
            for c in text.chars() {
                let part = alloc_str(c.encode_utf8(&mut buf));
                rnx_array_push(out, part as u64, 8);
            }
            return out;
        }
        let mut start = 0usize;
        for (i, _) in text.match_indices(nd_s) {
            let part = str_from_bytes(&text.as_bytes()[start..i]);
            rnx_array_push(out, part as u64, 8);
            start = i + nd_s.len();
        }
        let part = str_from_bytes(&text.as_bytes()[start..]);
        rnx_array_push(out, part as u64, 8);
        out
    }
}

pub(crate) fn str_from_bytes(bytes: &[u8]) -> *mut u8 {
    let out = str_alloc(bytes.len());
    if out.is_null() {
        return out;
    }
    unsafe {
        if !bytes.is_empty() {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), out.add(STR_HEADER), bytes.len());
        }
        out.add(STR_HEADER + bytes.len()).write(0);
    }
    out
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_string_trim(s: *const u8) -> *mut u8 {
    alloc_str(native_str(s).trim())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_string_char_code_at(s: *const u8, index: i64) -> i64 {
    if index < 0 {
        return -1;
    }
    native_str(s).chars().nth(index as usize).map(|c| c as i64).unwrap_or(-1)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_string_from_char_code(code: i64) -> *mut u8 {
    if code < 0 {
        return alloc_str("");
    }
    match char::from_u32(code as u32) {
        Some(c) => {
            let mut buf = [0u8; 4];
            alloc_str(c.encode_utf8(&mut buf))
        }
        None => alloc_str(""),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_str_free(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        rnx_free(ptr, str_body_size(str_len(ptr)));
    }
}

#[cfg(test)]
mod string_tests {
    use super::*;

    fn mortal(text: &str) -> *mut u8 {
        let bytes = text.as_bytes();
        let out = str_alloc(bytes.len());
        assert!(!out.is_null());
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), out.add(STR_HEADER), bytes.len());
            out.add(STR_HEADER).add(bytes.len()).write(0);
        }
        out
    }

    fn size_of(out: *mut u8) -> usize {
        str_body_size(str_len(out))
    }

    #[test]
    fn concat_joins_and_terminates() {
        unsafe {
            let a = mortal("Hello, ");
            let b = mortal("World!");
            let c = rnx_string_concat(a, b);
            assert!(!c.is_null());
            assert_eq!(str_bytes(c), b"Hello, World!");
            assert_eq!(*c.add(STR_HEADER).add(13), 0);
            rnx_release(a, size_of(a), None);
            rnx_release(b, size_of(b), None);
            rnx_release(c, size_of(c), None);
        }
    }

    #[test]
    fn concat_empty() {
        unsafe {
            let a = mortal("");
            let b = mortal("x");
            let c = rnx_string_concat(a, b);
            assert_eq!(str_bytes(c), b"x");
            rnx_release(a, size_of(a), None);
            rnx_release(b, size_of(b), None);
            rnx_release(c, size_of(c), None);
        }
    }

    #[test]
    fn method_len_counts_chars() {
        unsafe {
            let s = mortal("hllo");
            assert_eq!(rnx_string_len(s), 4);
            let u = mortal("hllo");
            assert_eq!(native_str(rnx_string_slice(u, 1, 3)), "ll");
            rnx_release(s, size_of(s), None);
            rnx_release(u, size_of(u), None);
        }
    }

    #[test]
    fn method_slice_clamps_and_empties() {
        unsafe {
            let s = mortal("hello");
            let r = |lo: i64, hi: i64| {
                let p = rnx_string_slice(s, lo, hi);
                let out = native_str(p);
                rnx_release(p, size_of(p), None);
                out
            };
            assert_eq!(r(0, 5), "hello");
            assert_eq!(r(1, 3), "el");
            assert_eq!(r(-10, 100), "hello");
            assert_eq!(r(3, 3), "");
            assert_eq!(r(4, 2), "");
            rnx_release(s, size_of(s), None);
        }
    }

    #[test]
    fn multibyte_slice_and_index() {
        unsafe {
            let s = mortal("héllo wörld 日本語テキスト");
            assert_eq!(rnx_string_len(s), 19);
            let a = rnx_string_slice(s, 0, 5);
            assert_eq!(native_str(a), "héllo");
            rnx_release(a, size_of(a), None);
            let b = rnx_string_slice(s, 6, 11);
            assert_eq!(native_str(b), "wörld");
            rnx_release(b, size_of(b), None);
            let nd = mortal("wörld");
            assert_eq!(rnx_string_index_of(s, nd), 6);
            let jp = mortal("日本語");
            assert_eq!(rnx_string_index_of_from(s, jp, 3), 12);
            let zz = mortal("zz");
            assert_eq!(rnx_string_index_of(s, zz), -1);
            let all = rnx_string_slice(s, 0, 100);
            assert_eq!(native_str(all), "héllo wörld 日本語テキスト");
            rnx_release(all, size_of(all), None);
            rnx_release(s, size_of(s), None);
            rnx_release(nd, size_of(nd), None);
            rnx_release(jp, size_of(jp), None);
            rnx_release(zz, size_of(zz), None);
        }
    }

    #[test]
    fn advancing_indexof_and_slice_scale_linearly() {
        unsafe {
            // Sizes are chosen so each measured pass is tens of milliseconds,
            // far above allocator and cache noise, and every timing is the
            // minimum of several passes. A single sub-millisecond sample cannot
            // support a doubling ratio bound: the noise alone exceeds it.
            const REPEATS: usize = 5;
            let unit = "abcdefghijklmnopqrstuvwxyz0123456789";
            let s100 = mortal(&unit.repeat(17_760));
            let s200 = mortal(&unit.repeat(35_520));
            let nd = mortal("9a");
            let drive_index = |s: *mut u8| {
                let mut hits = 0usize;
                let mut best = std::time::Duration::MAX;
                for _ in 0..REPEATS {
                    let t0 = std::time::Instant::now();
                    let mut from = 0i64;
                    let mut n = 0usize;
                    loop {
                        let at = rnx_string_index_of_from(s, nd, from);
                        if at < 0 {
                            break;
                        }
                        n += 1;
                        from = at + 1;
                    }
                    let d = std::time::Instant::now() - t0;
                    hits = n;
                    best = best.min(d);
                }
                (best, hits)
            };
            let (d100, h100) = drive_index(s100);
            let (d200, h200) = drive_index(s200);
            assert!(h100 > 2000, "{h100}");
            assert_eq!(h200, 2 * h100 + 1, "{h100} {h200}");
            let ratio = d200.as_secs_f64() / d100.as_secs_f64().max(1e-6);
            assert!(ratio < 2.6, "indexOf scaling ratio {ratio} ({d100:?} vs {d200:?})");
            let drive_slice = |s: *mut u8, total: i64| {
                let mut acc = 0i64;
                let mut best = std::time::Duration::MAX;
                for _ in 0..REPEATS {
                    let t0 = std::time::Instant::now();
                    let mut k = 0i64;
                    let mut a = 0i64;
                    while k + 64 <= total {
                        let w = rnx_string_slice(s, k, k + 64);
                        a += rnx_string_len(w);
                        rnx_release(w, size_of(w), None);
                        k += 64;
                    }
                    acc = a;
                    best = best.min(std::time::Instant::now() - t0);
                }
                (best, acc)
            };
            let n100 = rnx_string_len(s100);
            let n200 = rnx_string_len(s200);
            let (s100d, a100) = drive_slice(s100, n100);
            let (s200d, a200) = drive_slice(s200, n200);
            assert_eq!(a100, n100 / 64 * 64, "{a100} {n100}");
            assert_eq!(a200, n200 / 64 * 64, "{a200} {n200}");
            let sratio = s200d.as_secs_f64() / s100d.as_secs_f64().max(1e-6);
            assert!(sratio < 2.6, "slice scaling ratio {sratio} ({s100d:?} vs {s200d:?})");
            rnx_release(s100, size_of(s100), None);
            rnx_release(s200, size_of(s200), None);
            rnx_release(nd, size_of(nd), None);
        }
    }

    #[test]
    fn process_spawn_wait_kill() {
        let h = process_spawn_impl("echo", &["hello_proc".to_string()], "", &[], 2, 1, 2);
        assert!(h > 0);
        assert!(process_pid_of_impl(h) > 0);
        assert_eq!(process_wait_impl(h), 0);
        assert_eq!(process_exit_code_impl(h), 0);
        process_forget_impl(h);
        assert_eq!(process_pid_of_impl(h), 0);
    }

    #[test]
    fn process_run_captures_stdout() {
        let h = process_run_impl("echo", &["captured_123".to_string()], "", &[], 2, 1, 2);
        assert!(h > 0);
        assert_eq!(process_exit_code_impl(h), 0);
        let out = process_take_pipe_impl(h, 1);
        assert!(out != 0);
        let text = bytes_read_string_impl(out as *mut u8, 0, bytes_len_impl(out as *mut u8));
        assert!(text.contains("captured_123"), "{text}");
        bytes_free_impl(out as *mut u8);
        process_forget_impl(h);
    }

    #[test]
    fn process_kill_sleep() {
        let h = process_spawn_impl("sleep", &["30".to_string()], "", &[], 2, 2, 2);
        assert!(h > 0);
        assert_eq!(process_kill_impl(h, 9), 1);
        let code = process_wait_impl(h);
        assert_eq!(code, 128 + 9, "{code}");
        process_forget_impl(h);
    }

    #[test]
    fn process_no_fd_leak_over_runs() {
        let fds_before = std::fs::read_dir("/proc/self/fd").map(|d| d.count()).unwrap_or(0);
        for _ in 0..50 {
            let h = process_run_impl("echo", &["x".to_string()], "", &[], 2, 1, 2);
            assert_eq!(process_exit_code_impl(h), 0);
            let out = process_take_pipe_impl(h, 1);
            assert!(out != 0);
            bytes_free_impl(out as *mut u8);
            process_forget_impl(h);
        }
        let fds_after = std::fs::read_dir("/proc/self/fd").map(|d| d.count()).unwrap_or(0);
        assert!(fds_after <= fds_before + 2, "fds {fds_before} -> {fds_after}");
    }

    #[test]
    fn os_queries_sane() {
        assert_eq!(os_string("RNX_DEFINITELY_UNSET_VAR_XYZ"), "");
        assert!(os_uptime_secs() >= 0.0);
        assert!(std::thread::available_parallelism().is_ok());
    }

    #[test]
    fn method_index_of_char_offsets() {
        unsafe {
            let s = mortal("hello world");
            let n = mortal("world");
            let m = mortal("nope");
            let e = mortal("");
            assert_eq!(rnx_string_index_of(s, n), 6);
            assert_eq!(rnx_string_index_of(s, m), -1);
            assert_eq!(rnx_string_index_of(s, e), 0);
            assert_eq!(rnx_string_index_of_from(s, n, 0), 6);
            assert_eq!(rnx_string_index_of_from(s, n, 7), -1);
            assert_eq!(rnx_string_index_of_from(s, m, 0), -1);
            assert_eq!(rnx_string_index_of_from(s, e, 4), 4);
            assert_eq!(rnx_string_index_of_from(s, n, 99), -1);
            assert_eq!(rnx_string_index_of_from(s, n, -5), 6);
            let u = mortal("héllo wörld");
            let un = mortal("llo");
            let uo = mortal("ö");
            assert_eq!(rnx_string_index_of_from(u, un, 0), 2);
            assert_eq!(rnx_string_index_of_from(u, un, 3), -1);
            assert_eq!(rnx_string_index_of_from(u, uo, 0), 7);
            assert_eq!(rnx_string_index_of_from(u, uo, 8), -1);
            for p in [u, un, uo] {
                rnx_release(p, size_of(p), None);
            }
            for p in [s, n, m, e] {
                rnx_release(p, size_of(p), None);
            }
        }
    }

    #[test]
    fn method_trim_and_char_code() {
        unsafe {
            let s = mortal("  a  ");
            let t = rnx_string_trim(s);
            assert_eq!(native_str(t), "a");
            assert_eq!(rnx_string_char_code_at(t, 0), 97);
            assert_eq!(rnx_string_char_code_at(t, 5), -1);
            assert_eq!(rnx_string_char_code_at(t, -1), -1);
            rnx_release(s, size_of(s), None);
            rnx_release(t, size_of(t), None);
        }
    }

    #[test]
    fn eq_semantics() {
        unsafe {
            let a = mortal("abc");
            let b = mortal("abc");
            let c = mortal("abd");
            let d = mortal("ab");
            assert!(rnx_string_eq(a, a));
            assert!(rnx_string_eq(a, b));
            assert!(!rnx_string_eq(a, c));
            assert!(!rnx_string_eq(a, d));
            assert!(!rnx_string_eq(a, std::ptr::null()));
            assert!(rnx_string_eq(std::ptr::null(), std::ptr::null()));
            for p in [a, b, c, d] {
                rnx_release(p, size_of(p), None);
            }
        }
    }

    #[test]
    fn immortal_skipped() {
        let mut buf = vec![0u8; STR_HEADER + 4];
        let p = buf.as_mut_ptr();
        unsafe {
            (p as *mut u32).write_unaligned(IMMORTAL);
            ((p.add(16)) as *mut u64).write_unaligned(3);
            std::ptr::copy_nonoverlapping(b"imm".as_ptr(), p.add(STR_HEADER), 3);
            rnx_retain(p);
            rnx_release(p, STR_HEADER + 4, None);
            assert_eq!((p as *const u32).read_unaligned(), IMMORTAL);
            assert!(rnx_string_eq(p, p));
        }
    }

    #[test]
    fn int_to_str_values() {
        for (v, want) in [(0, "0"), (42, "42"), (-7, "-7"), (i64::MIN, "-9223372036854775808")] {
            let p = unsafe { rnx_int_to_str(v) };
            assert!(!p.is_null());
            assert_eq!(str_bytes(p), want.as_bytes());
            unsafe { rnx_release(p, str_body_size(str_len(p)), None) };
        }
    }

    #[test]
    fn print_val_runs() {
        unsafe {
            rnx_print_val(42, TAG_INT);
            rnx_print_val(1, TAG_BOOL);
            rnx_print_val(0, TAG_BOOL);
            rnx_print_val(3.0f64.to_bits(), TAG_FLOAT);
            let s = mortal("hi");
            rnx_print_val(s as u64, TAG_STR);
            rnx_print_val(0, TAG_STR);
            let arr = crate::native::rnx_array_new(0, 8);
            assert!(!arr.is_null());
            crate::native::rnx_heap_track(arr, crate::native::HEAP_ARRAY, 8, 0);
            crate::native::pretty::rnx_note_array_kind(arr, crate::native::pretty::KIND_INT, 0);
            rnx_print_val(arr as u64, TAG_PTR);
            crate::native::rnx_any_release(arr as u64);
            rnx_print_val(0, TAG_PTR);
            rnx_release(s, size_of(s), None);
        }
    }
}

#[cfg(test)]
mod release_str_tests {
    use super::*;

    #[test]
    fn release_str_null_safe() {
        unsafe {
            rnx_release_str(std::ptr::null_mut());
        }
    }

    #[test]
    fn release_str_frees() {
        unsafe {
            let p = rnx_int_to_str(12345);
            assert!(!p.is_null());
            rnx_release_str(p);
        }
    }

    #[test]
    fn release_str_immortal() {
        let mut buf = vec![0u8; STR_HEADER + 4];
        let p = buf.as_mut_ptr();
        unsafe {
            (p as *mut u32).write_unaligned(IMMORTAL);
            rnx_release_str(p);
            assert_eq!((p as *const u32).read_unaligned(), IMMORTAL);
        }
    }
}
