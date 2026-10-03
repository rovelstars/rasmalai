pub struct StructuralIndex {
    pub offsets: Vec<u32>,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct ScanCarry {
    in_string: bool,
    ends_backslash: bool,
    trailing_odd: bool,
}

pub fn scan_structurals(input: &[u8]) -> StructuralIndex {
    let mut out = Vec::with_capacity(input.len() / 8);
    let mut cur = StreamCursor::default();
    while stream_next(input, &mut cur, &mut out) {}
    StructuralIndex { offsets: out }
}

#[derive(Default)]
pub(crate) struct StreamCursor {
    pos: usize,
    carry: ScanCarry,
}

pub(crate) fn stream_next(input: &[u8], cur: &mut StreamCursor, out: &mut Vec<u32>) -> bool {
    if cur.pos >= input.len() {
        return false;
    }
    let end = (cur.pos + 16).min(input.len());
    let block = &input[cur.pos..end];
    if block.len() == 16 {
        scan_chunk(block, cur.pos as u32, &mut cur.carry, out);
    } else {
        scan_tail(block, cur.pos as u32, &mut cur.carry, out);
    }
    cur.pos = end;
    true
}

pub(crate) fn count_structurals(input: &[u8]) -> usize {
    let mut cur = StreamCursor::default();
    let mut tmp = Vec::with_capacity(32);
    let mut n = 0usize;
    while stream_next(input, &mut cur, &mut tmp) {
        n += tmp.len();
        tmp.clear();
    }
    n
}

#[cfg(all(target_arch = "x86_64", not(target_arch = "wasm32")))]
fn scan_chunk(block: &[u8], base: u32, carry: &mut ScanCarry, out: &mut Vec<u32>) {
    unsafe {
        use std::arch::x86_64::*;
        let v = _mm_loadu_si128(block.as_ptr() as *const __m128i);
        let q = _mm_cmpeq_epi8(v, _mm_set1_epi8(b'"' as i8));
        let b = _mm_cmpeq_epi8(v, _mm_set1_epi8(b'\\' as i8));
        let c1 = _mm_cmpeq_epi8(v, _mm_set1_epi8(b'{' as i8));
        let c2 = _mm_cmpeq_epi8(v, _mm_set1_epi8(b'}' as i8));
        let c3 = _mm_cmpeq_epi8(v, _mm_set1_epi8(b'[' as i8));
        let c4 = _mm_cmpeq_epi8(v, _mm_set1_epi8(b']' as i8));
        let c5 = _mm_cmpeq_epi8(v, _mm_set1_epi8(b':' as i8));
        let c6 = _mm_cmpeq_epi8(v, _mm_set1_epi8(b',' as i8));
        let qm = _mm_movemask_epi8(q) as u32 as u16;
        let bm = _mm_movemask_epi8(b) as u32 as u16;
        let sm = _mm_movemask_epi8(c1) as u32 as u16
            | _mm_movemask_epi8(c2) as u32 as u16
            | _mm_movemask_epi8(c3) as u32 as u16
            | _mm_movemask_epi8(c4) as u32 as u16
            | _mm_movemask_epi8(c5) as u32 as u16
            | _mm_movemask_epi8(c6) as u32 as u16;
        let (real_q, next) = classify_masks(qm, bm, *carry);
        emit_chunk(base, sm, real_q, &next, carry, out);
    }
}

#[cfg(any(not(target_arch = "x86_64"), target_arch = "wasm32"))]
fn scan_chunk(block: &[u8], base: u32, carry: &mut ScanCarry, out: &mut Vec<u32>) {
    scan_tail(block, base, carry, out);
}

fn classify_masks(qm: u16, bm: u16, carry: ScanCarry) -> (u16, ScanCarry) {
    let mut starts = bm & !(bm << 1);
    if carry.ends_backslash {
        starts &= !1;
    }
    let mut even = starts;
    even |= (even << 2) & bm;
    even |= (even << 4) & bm;
    even |= (even << 8) & bm;
    if carry.ends_backslash {
        let run = (!bm).trailing_zeros();
        let lead = if run >= 16 { 0xFFFFu16 } else { ((1u32 << run) - 1) as u16 };
        even |= lead & if carry.trailing_odd { 0xAAAA } else { 0x5555 };
    }
    let end_run = bm.leading_ones();
    let (ends_bs, trailing_odd) = if end_run >= 16 {
        (true, carry.trailing_odd)
    } else {
        (end_run > 0, end_run & 1 == 1)
    };
    let start_esc = (carry.ends_backslash && carry.trailing_odd) as u16;
    let esc = qm & ((even << 1) | start_esc);
    let real_q = qm & !esc;
    let in_string = carry.in_string ^ (real_q.count_ones() & 1 == 1);
    let next = ScanCarry {
        in_string,
        ends_backslash: ends_bs,
        trailing_odd,
    };
    (real_q, next)
}

fn emit_chunk(
    base: u32,
    sm: u16,
    real_q: u16,
    next: &ScanCarry,
    carry: &mut ScanCarry,
    out: &mut Vec<u32>,
) {
    let in_start = carry.in_string;
    if real_q == 0 {
        if !in_start {
            let mut m = sm;
            while m != 0 {
                let i = m.trailing_zeros();
                out.push(base + i);
                m &= m - 1;
            }
        }
        *carry = *next;
        return;
    }
    let mut inside = in_start;
    let mut cand = sm | real_q;
    let mut qi = real_q;
    while cand != 0 {
        let i = cand.trailing_zeros();
        let bit = 1u16 << i;
        if qi & bit != 0 {
            inside = !inside;
            qi &= !bit;
        } else if !inside {
            out.push(base + i);
        }
        cand &= cand - 1;
    }
    debug_assert_eq!(inside, next.in_string);
    *carry = *next;
}

fn scan_tail(block: &[u8], base: u32, carry: &mut ScanCarry, out: &mut Vec<u32>) {
    let mut escaped = carry.ends_backslash && carry.trailing_odd;
    for (i, &b) in block.iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        if carry.in_string {
            if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                carry.in_string = false;
            }
            continue;
        }
        match b {
            b'"' => carry.in_string = true,
            b'{' | b'}' | b'[' | b']' | b':' | b',' => out.push(base + i as u32),
            _ => {}
        }
    }
    carry.ends_backslash = false;
    carry.trailing_odd = false;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn structs_of(s: &str) -> Vec<u32> {
        scan_structurals(s.as_bytes()).offsets
    }

    #[test]
    fn empty_and_plain() {
        assert!(structs_of("").is_empty());
        assert!(structs_of("   ").is_empty());
        assert!(structs_of("123").is_empty());
    }

    #[test]
    fn object_boundaries() {
        assert_eq!(structs_of("{}"), vec![0, 1]);
        assert_eq!(structs_of("{\"a\":1}"), vec![0, 4, 6]);
    }

    #[test]
    fn strings_hide_structurals() {
        assert_eq!(structs_of("\"{[,]}\""), Vec::<u32>::new());
        assert_eq!(structs_of("[\"a,b\",{\"c\":}]"), vec![0, 6, 7, 11, 12, 13]);
    }

    #[test]
    fn escaped_quotes_branch_free() {
        assert_eq!(structs_of("\"a\\\"{b\""), Vec::<u32>::new());
        assert_eq!(structs_of("{\"q\":\"a\\\"b\\\\c\"}"), vec![0, 4, 14]);
        assert_eq!(structs_of("\"\\\\\\\\\""), Vec::<u32>::new());
        assert_eq!(structs_of("[\"\\\\\\\\\"},]"), vec![0, 7, 8, 9]);
    }

    #[test]
    fn escapes_across_chunk_edge() {
        let pad = " ".repeat(15);
        let doc = format!("{pad}\"\\\\\"{pad},");
        let idx = structs_of(&doc);
        assert_eq!(idx, vec![(15 + 4 + 15) as u32]);
        let doc2 = format!("{pad}\"a\\\"\",{pad}");
        let idx2 = structs_of(&doc2);
        assert_eq!(idx2, vec![20]);
    }

    #[test]
    fn long_run_matches_scalar() {
        let mut doc = String::from("[");
        for i in 0..64 {
            if i > 0 {
                doc.push(',');
            }
            doc.push_str("{\"id\":");
            doc.push_str(&i.to_string());
            doc.push_str(",\"v\":\"x\\\\\\\"y\"}");
        }
        doc.push(']');
        let v = scan_structurals(doc.as_bytes()).offsets;
        let mut ref_out = Vec::new();
        let mut carry = ScanCarry::default();
        scan_tail(doc.as_bytes(), 0, &mut carry, &mut ref_out);
        assert_eq!(v, ref_out);
    }

    #[test]
    fn fuzz_against_scalar() {
        let keys = ["a", "id", "x y", "q\"", "bs\\", "uni", "k,}", "{\"", "longkeyname"];
        let vals = ["a b", "x\"y", "p\\q", "a,b:c{}[]", "", "tab\there", "nl\nhere"];
        let mut state = 0x12345678u64;
        let mut next = move || {
            state = state
                .wrapping_mul(0x5851F42D4C957F2D)
                .wrapping_add(0x14057B7EF767814F);
            (state >> 33) as usize
        };
        fn esc(s: &str) -> String {
            let mut o = String::new();
            for c in s.chars() {
                match c {
                    '"' => o.push_str("\\\""),
                    '\\' => o.push_str("\\\\"),
                    '\t' => o.push_str("\\t"),
                    '\n' => o.push_str("\\n"),
                    _ => o.push(c),
                }
            }
            o
        }
        fn gen_doc(depth: usize, next: &mut dyn FnMut() -> usize, keys: &[&str], vals: &[&str]) -> String {
            let kind = next() % if depth == 0 { 5 } else { 7 };
            match kind {
                0 => "null".to_string(),
                1 => ["true", "false"][next() % 2].to_string(),
                2 => format!("{}", (next() % 200000) as i64 - 100000),
                3 => format!("{}", (next() % 10000) as f64 / 7.0),
                4 => format!("\"{}\"", esc(vals[next() % vals.len()])),
                5 => {
                    let n = next() % 4;
                    let mut o = String::from("{");
                    for i in 0..n {
                        if i > 0 {
                            o.push(',');
                        }
                        o.push('"');
                        o.push_str(&esc(keys[next() % keys.len()]));
                        o.push_str("\":");
                        o.push_str(&gen_doc(depth + 1, next, keys, vals));
                    }
                    o.push('}');
                    o
                }
                _ => {
                    let n = next() % 4;
                    let mut o = String::from("[");
                    for i in 0..n {
                        if i > 0 {
                            o.push(',');
                        }
                        o.push_str(&gen_doc(depth + 1, next, keys, vals));
                    }
                    o.push(']');
                    o
                }
            }
        }
        for round in 0..500 {
            let doc = gen_doc(0, &mut next, &keys, &vals);
            let bytes = doc.as_bytes();
            let simd = scan_structurals(bytes).offsets;
            let mut ref_out = Vec::new();
            let mut carry = ScanCarry::default();
            scan_tail(bytes, 0, &mut carry, &mut ref_out);
            assert_eq!(simd, ref_out, "round {round} doc={doc:?}");
        }
    }
}
