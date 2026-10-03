use std::sync::Arc;

use super::json_scanner::{count_structurals, stream_next, StreamCursor};
use super::JsonSink;

pub const TAPE_NULL: u8 = 0;
pub const TAPE_BOOL: u8 = 1;
pub const TAPE_INT: u8 = 2;
pub const TAPE_FLOAT: u8 = 3;
pub const TAPE_STR: u8 = 4;
pub const TAPE_ARRAY: u8 = 5;
pub const TAPE_OBJECT: u8 = 6;
pub const TAPE_KEY: u8 = 7;

pub const TAPE_MAX_DEPTH: usize = 64;

pub const TAPE_INT_WIDE: u32 = 0x80_0000;

#[derive(Debug)]
pub struct JsonError {
    pub msg: String,
    pub offset: usize,
    pub is_depth: bool,
}

impl JsonError {
    pub fn new(msg: &str, offset: usize) -> JsonError {
        JsonError {
            msg: msg.to_string(),
            offset,
            is_depth: false,
        }
    }

    fn depth() -> JsonError {
        JsonError {
            msg: "json max depth exceeded".to_string(),
            offset: 0,
            is_depth: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct JsonDoc {
    pub nodes: Vec<u64>,
    pub strings: Vec<u8>,
    pub keys: Vec<u8>,
    pub key_ends: Vec<u32>,
}

pub fn tape_tag(word: u64) -> u8 {
    word as u8
}

pub fn tape_aux(word: u64) -> u32 {
    (word >> 8) as u32 & 0xFF_FFFF
}

pub fn tape_payload(word: u64) -> u32 {
    (word >> 32) as u32
}

fn mk_word(tag: u8, aux: u32, payload: u32) -> u64 {
    tag as u64 | ((aux & 0xFF_FFFF) as u64) << 8 | (payload as u64) << 32
}

pub fn node_words(nodes: &[u64], idx: usize) -> usize {
    match tape_tag(nodes[idx]) {
        TAPE_INT => {
            if tape_aux(nodes[idx]) & TAPE_INT_WIDE != 0 {
                2
            } else {
                1
            }
        }
        TAPE_FLOAT => 2,
        TAPE_KEY => 1,
        TAPE_STR | TAPE_NULL | TAPE_BOOL => 1,
        TAPE_ARRAY => {
            let mut n = idx + 1;
            let count = tape_aux(nodes[idx]) as usize;
            for _ in 0..count {
                n += node_words(nodes, n);
            }
            n - idx
        }
        TAPE_OBJECT => {
            let mut n = idx + 1;
            let pairs = tape_aux(nodes[idx]) as usize;
            for _ in 0..pairs {
                n += 1;
                n += node_words(nodes, n);
            }
            n - idx
        }
        _ => 1,
    }
}

pub fn array_count(nodes: &[u64], idx: usize) -> usize {
    tape_aux(nodes[idx]) as usize
}

pub fn object_count(nodes: &[u64], idx: usize) -> usize {
    tape_aux(nodes[idx]) as usize
}

pub fn array_elem_at(nodes: &[u64], idx: usize, elem: usize) -> usize {
    let mut n = idx + 1;
    for _ in 0..elem {
        n += node_words(nodes, n);
    }
    n
}

pub struct ArrayElems<'a> {
    nodes: &'a [u64],
    cur: usize,
    left: usize,
}

impl<'a> Iterator for ArrayElems<'a> {
    type Item = usize;
    fn next(&mut self) -> Option<usize> {
        if self.left == 0 {
            return None;
        }
        let out = self.cur;
        self.cur += node_words(self.nodes, self.cur);
        self.left -= 1;
        Some(out)
    }
}

pub fn array_iter(nodes: &[u64], idx: usize) -> ArrayElems<'_> {
    ArrayElems {
        nodes,
        cur: idx + 1,
        left: array_count(nodes, idx),
    }
}

pub struct ObjectPairs<'a> {
    nodes: &'a [u64],
    cur: usize,
    left: usize,
}

impl<'a> Iterator for ObjectPairs<'a> {
    type Item = (usize, usize);
    fn next(&mut self) -> Option<(usize, usize)> {
        if self.left == 0 {
            return None;
        }
        let k = self.cur;
        let v = k + 1;
        self.cur = v + node_words(self.nodes, v);
        self.left -= 1;
        Some((k, v))
    }
}

pub fn object_iter(nodes: &[u64], idx: usize) -> ObjectPairs<'_> {
    ObjectPairs {
        nodes,
        cur: idx + 1,
        left: object_count(nodes, idx),
    }
}

pub fn object_key_at(nodes: &[u64], idx: usize, pair: usize) -> usize {
    let mut n = idx + 1;
    for _ in 0..pair {
        n += 1;
        n += node_words(nodes, n);
    }
    n
}

pub fn object_val_at(nodes: &[u64], idx: usize, pair: usize) -> usize {
    object_key_at(nodes, idx, pair) + 1
}

pub fn str_at<'a>(doc: &'a JsonDoc, idx: usize) -> &'a [u8] {
    let off = tape_payload(doc.nodes[idx]) as usize;
    let len = tape_aux(doc.nodes[idx]) as usize;
    &doc.strings[off..off + len]
}

pub fn key_at<'a>(doc: &'a JsonDoc, idx: usize) -> &'a [u8] {
    let k = tape_payload(doc.nodes[idx]) as usize;
    let end = doc.key_ends[k] as usize;
    let start = if k == 0 { 0 } else { doc.key_ends[k - 1] as usize };
    &doc.keys[start..end]
}

pub fn intern_key(doc: &mut JsonDoc, bytes: &[u8]) -> u32 {
    let mut start = 0usize;
    for (i, end) in doc.key_ends.iter().enumerate() {
        let end = *end as usize;
        if tape_key_eq(&doc.keys[start..end], bytes) {
            return i as u32;
        }
        start = end;
    }
    doc.keys.extend_from_slice(bytes);
    doc.key_ends.push(doc.keys.len() as u32);
    doc.key_ends.len() as u32 - 1
}

pub fn int_at(doc: &JsonDoc, idx: usize) -> i64 {
    if tape_aux(doc.nodes[idx]) & TAPE_INT_WIDE != 0 {
        doc.nodes[idx + 1] as i64
    } else {
        tape_payload(doc.nodes[idx]) as u32 as i32 as i64
    }
}

pub fn float_at(doc: &JsonDoc, idx: usize) -> f64 {
    f64::from_bits(doc.nodes[idx + 1])
}

pub fn bool_at(doc: &JsonDoc, idx: usize) -> bool {
    tape_payload(doc.nodes[idx]) != 0
}

pub fn tape_key_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let n = a.len();
    let mut i = 0;
    while i + 8 <= n {
        let x = u64::from_le_bytes([a[i], a[i + 1], a[i + 2], a[i + 3], a[i + 4], a[i + 5], a[i + 6], a[i + 7]]);
        let y = u64::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3], b[i + 4], b[i + 5], b[i + 6], b[i + 7]]);
        if x != y {
            return false;
        }
        i += 8;
    }
    while i < n {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

pub fn find_key(doc: &JsonDoc, obj: usize, key: &[u8]) -> Option<usize> {
    for (k, v) in object_iter(&doc.nodes, obj) {
        if tape_key_eq(key_at(doc, k), key) {
            return Some(v);
        }
    }
    None
}

struct StructFeed<'a> {
    input: &'a [u8],
    cur: StreamCursor,
    pending: Vec<u32>,
    base_si: usize,
    done: bool,
}

impl<'a> StructFeed<'a> {
    fn get(&mut self, si: usize) -> Option<u32> {
        while !self.done && self.base_si + self.pending.len() <= si {
            if !stream_next(self.input, &mut self.cur, &mut self.pending) {
                self.done = true;
            }
        }
        if si < self.base_si {
            return None;
        }
        self.pending.get(si - self.base_si).copied()
    }

    fn compact(&mut self, si: usize) {
        if si >= self.base_si + 8192 {
            let drop = si - self.base_si;
            self.pending.drain(..drop);
            self.base_si = si;
        }
    }
}

#[derive(Default)]
struct FloatCache {
    slots: [Option<([u8; 40], usize, u64)>; 8],
}

impl FloatCache {
    fn get(&self, text: &[u8]) -> Option<f64> {
        if text.len() > 40 {
            return None;
        }
        for s in self.slots.iter().flatten() {
            if s.1 == text.len() && s.0[..s.1] == *text {
                return Some(f64::from_bits(s.2));
            }
        }
        None
    }

    fn put(&mut self, text: &[u8], f: f64) {
        if text.len() > 40 {
            return;
        }
        let mut key = [0u8; 40];
        key[..text.len()].copy_from_slice(text);
        for i in (1..8).rev() {
            self.slots[i] = self.slots[i - 1].take();
        }
        self.slots[0] = Some((key, text.len(), f.to_bits()));
    }
}

struct Builder<'a> {
    input: &'a [u8],
    feed: StructFeed<'a>,
    si: usize,
    pos: usize,
    nodes: Vec<u64>,
    strings: Vec<u8>,
    keys: Vec<u8>,
    key_ends: Vec<u32>,
    floats: FloatCache,
}

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}

impl<'a> Builder<'a> {
    fn err(&self, msg: &str) -> JsonError {
        JsonError::new(msg, self.pos)
    }

    fn skip_ws(&mut self) {
        while self.pos < self.input.len() && is_ws(self.input[self.pos]) {
            self.pos += 1;
        }
    }

    fn expect_struct(&mut self, ch: u8) -> Result<(), JsonError> {
        self.skip_ws();
        match self.feed.get(self.si) {
            Some(off) if off as usize == self.pos && self.input.get(self.pos) == Some(&ch) => {
                self.pos += 1;
                self.si += 1;
                self.feed.compact(self.si);
                Ok(())
            }
            _ => Err(self.err("expected structural character")),
        }
    }

    fn peek_struct(&mut self) -> Option<u8> {
        self.skip_ws();
        match self.feed.get(self.si) {
            Some(off) if off as usize == self.pos => self.input.get(self.pos).copied(),
            _ => None,
        }
    }

    fn parse_value(&mut self, depth: usize) -> Result<usize, JsonError> {
        if depth >= TAPE_MAX_DEPTH {
            return Err(JsonError::depth());
        }
        self.skip_ws();
        let c = *self.input.get(self.pos).ok_or_else(|| self.err("unexpected end"))?;
        match c {
            b'{' => self.parse_object(depth),
            b'[' => self.parse_array(depth),
            b'"' => self.parse_string(),
            b't' => self.parse_literal("true"),
            b'f' => self.parse_literal("false"),
            b'n' => self.parse_literal("null"),
            b'-' | b'0'..=b'9' => self.parse_number(),
            _ => Err(self.err("unexpected character")),
        }
    }

    fn parse_literal(&mut self, lit: &str) -> Result<usize, JsonError> {
        let bytes = lit.as_bytes();
        if self.input.len() < self.pos + bytes.len() || &self.input[self.pos..self.pos + bytes.len()] != bytes {
            return Err(self.err("invalid literal"));
        }
        self.pos += bytes.len();
        let idx = self.nodes.len();
        match lit {
            "true" => self.nodes.push(mk_word(TAPE_BOOL, 0, 1)),
            "false" => self.nodes.push(mk_word(TAPE_BOOL, 0, 0)),
            _ => self.nodes.push(mk_word(TAPE_NULL, 0, 0)),
        }
        Ok(idx)
    }

    fn parse_number(&mut self) -> Result<usize, JsonError> {
        let start = self.pos;
        let len = self.input.len();
        let mut p = self.pos;
        let neg = self.input.get(p) == Some(&b'-');
        if neg {
            p += 1;
        }
        let int_start = p;
        let mut mag: u64 = 0;
        let mut mag_over = false;
        if p < len && self.input[p] == b'0' {
            p += 1;
        } else {
            while p < len && self.input[p].is_ascii_digit() {
                let d = (self.input[p] - b'0') as u64;
                match mag.checked_mul(10).and_then(|m| m.checked_add(d)) {
                    Some(m) => mag = m,
                    None => mag_over = true,
                }
                p += 1;
            }
            if p == int_start {
                return Err(self.err("invalid number"));
            }
        }
        let mut is_float = false;
        if p < len && self.input[p] == b'.' {
            is_float = true;
            p += 1;
            let frac = p;
            while p < len && self.input[p].is_ascii_digit() {
                p += 1;
            }
            if p == frac {
                return Err(self.err("invalid number"));
            }
        }
        if p < len && (self.input[p] == b'e' || self.input[p] == b'E') {
            is_float = true;
            p += 1;
            if p < len && (self.input[p] == b'+' || self.input[p] == b'-') {
                p += 1;
            }
            let exp = p;
            while p < len && self.input[p].is_ascii_digit() {
                p += 1;
            }
            if p == exp {
                return Err(self.err("invalid number"));
            }
        }
        let slice = &self.input[start..p];
        self.pos = p;
        let idx = self.nodes.len();
        if !is_float && !mag_over {
            let limit = i64::MAX as u64 + neg as u64;
            if mag <= limit {
                let n = if neg { (mag as i64).wrapping_neg() } else { mag as i64 };
                if n >= i32::MIN as i64 && n <= i32::MAX as i64 {
                    self.nodes.push(mk_word(TAPE_INT, 0, n as u32));
                } else {
                    self.nodes.push(mk_word(TAPE_INT, TAPE_INT_WIDE, 0));
                    self.nodes.push(n as u64);
                }
                return Ok(idx);
            }
        }
        let text = std::str::from_utf8(slice).map_err(|_| self.err("invalid number"))?;
        if !is_float {
            if let Ok(n) = text.parse::<i64>() {
                if n >= i32::MIN as i64 && n <= i32::MAX as i64 {
                    self.nodes.push(mk_word(TAPE_INT, 0, n as u32));
                } else {
                    self.nodes.push(mk_word(TAPE_INT, TAPE_INT_WIDE, 0));
                    self.nodes.push(n as u64);
                }
                return Ok(idx);
            }
        }
        if let Some(f) = self.floats.get(slice) {
            self.nodes.push(mk_word(TAPE_FLOAT, 0, 0));
            self.nodes.push(f.to_bits());
            return Ok(idx);
        }
        match text.parse::<f64>() {
            Ok(f) if f.is_finite() => {
                self.floats.put(slice, f);
                self.nodes.push(mk_word(TAPE_FLOAT, 0, 0));
                self.nodes.push(f.to_bits());
                Ok(idx)
            }
            _ => Err(self.err("invalid number")),
        }
    }

    fn parse_hex4(&mut self) -> Result<u32, JsonError> {
        if self.pos + 4 > self.input.len() {
            return Err(self.err("bad unicode escape"));
        }
        let mut v = 0u32;
        for i in 0..4 {
            let c = self.input[self.pos + i];
            let d = match c {
                b'0'..=b'9' => (c - b'0') as u32,
                b'a'..=b'f' => (c - b'a' + 10) as u32,
                b'A'..=b'F' => (c - b'A' + 10) as u32,
                _ => return Err(self.err("bad unicode escape")),
            };
            v = v * 16 + d;
        }
        self.pos += 4;
        Ok(v)
    }

    fn find_key_idx(&self, bytes: &[u8]) -> Option<u32> {
        let mut start = 0usize;
        for (i, end) in self.key_ends.iter().enumerate() {
            let end = *end as usize;
            if tape_key_eq(&self.keys[start..end], bytes) {
                return Some(i as u32);
            }
            start = end;
        }
        None
    }

    fn key_fp(&self, kidx: usize) -> (u64, usize) {
        let k = tape_payload(self.nodes[kidx]) as usize;
        let end = self.key_ends[k] as usize;
        let start = if k == 0 { 0 } else { self.key_ends[k - 1] as usize };
        let len = end - start;
        let mut be = 0u64;
        let n = len.min(8);
        for i in 0..n {
            be |= (self.keys[start + i] as u64) << (8 * (7 - i));
        }
        ((be << 1) | ((len > 8) as u64), kidx)
    }

    fn intern(&mut self, bytes: &[u8]) -> u32 {
        if let Some(i) = self.find_key_idx(bytes) {
            return i;
        }
        self.keys.extend_from_slice(bytes);
        self.key_ends.push(self.keys.len() as u32);
        self.key_ends.len() as u32 - 1
    }

    fn parse_key(&mut self) -> Result<usize, JsonError> {
        let off = self.strings.len();
        let kidx = self.parse_string()?;
        let (ki, klen) = match self.find_key_idx(&self.strings[off..]) {
            Some(i) => (i, self.strings.len() - off),
            None => {
                let bytes = self.strings[off..].to_vec();
                (self.intern(&bytes), bytes.len())
            }
        };
        self.strings.truncate(off);
        self.nodes.truncate(kidx);
        if klen > 0xFF_FFFF {
            return Err(self.err("key too long"));
        }
        let idx = self.nodes.len();
        self.nodes.push(mk_word(TAPE_KEY, klen as u32, ki));
        Ok(idx)
    }

    fn parse_string(&mut self) -> Result<usize, JsonError> {
        if self.input.get(self.pos) != Some(&b'"') {
            return Err(self.err("expected string"));
        }
        self.pos += 1;
        let off = self.strings.len();
        let mut simple = true;
        loop {
            let c = *self.input.get(self.pos).ok_or_else(|| self.err("unterminated string"))?;
            if c == b'"' {
                self.pos += 1;
                break;
            }
            if c == b'\\' {
                simple = false;
                break;
            }
            if c < 0x20 {
                return Err(self.err("unescaped control character"));
            }
            self.strings.push(c);
            self.pos += 1;
        }
        if !simple {
            loop {
                let c = *self.input.get(self.pos).ok_or_else(|| self.err("unterminated string"))?;
                if c == b'"' {
                    self.pos += 1;
                    break;
                }
                if c == b'\\' {
                    self.pos += 1;
                    let e = *self.input.get(self.pos).ok_or_else(|| self.err("unterminated string"))?;
                    match e {
                        b'"' => self.strings.push(b'"'),
                        b'\\' => self.strings.push(b'\\'),
                        b'/' => self.strings.push(b'/'),
                        b'b' => self.strings.push(0x08),
                        b'f' => self.strings.push(0x0C),
                        b'n' => self.strings.push(b'\n'),
                        b'r' => self.strings.push(b'\r'),
                        b't' => self.strings.push(b'\t'),
                        b'u' => {
                            self.pos += 1;
                            let mut cp = self.parse_hex4()?;
                            if (0xD800..0xDC00).contains(&cp) {
                                if self.input.get(self.pos) == Some(&b'\\')
                                    && self.input.get(self.pos + 1) == Some(&b'u')
                                {
                                    self.pos += 2;
                                    let lo = self.parse_hex4()?;
                                    if (0xDC00..0xE000).contains(&lo) {
                                        cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                                    } else {
                                        return Err(self.err("bad unicode escape"));
                                    }
                                } else {
                                    return Err(self.err("bad unicode escape"));
                                }
                            } else if (0xDC00..0xE000).contains(&cp) {
                                return Err(self.err("bad unicode escape"));
                            }
                            match char::from_u32(cp) {
                                Some(ch) => {
                                    let mut buf = [0u8; 4];
                                    self.strings.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                                    continue;
                                }
                                None => return Err(self.err("bad unicode escape")),
                            }
                        }
                        _ => return Err(self.err("bad escape")),
                    }
                    self.pos += 1;
                } else {
                    if c < 0x20 {
                        return Err(self.err("unescaped control character"));
                    }
                    self.strings.push(c);
                    self.pos += 1;
                }
            }
        }
        let slen = self.strings.len() - off;
        if slen > 0xFF_FFFF {
            return Err(self.err("string too long"));
        }
        let idx = self.nodes.len();
        self.nodes.push(mk_word(TAPE_STR, slen as u32, off as u32));
        Ok(idx)
    }

    fn parse_array(&mut self, depth: usize) -> Result<usize, JsonError> {
        self.expect_struct(b'[')?;
        let idx = self.nodes.len();
        self.nodes.push(mk_word(TAPE_ARRAY, 0, 0));
        let mut count = 0u32;
        if self.peek_struct() == Some(b']') {
            self.expect_struct(b']')?;
            return Ok(idx);
        }
        loop {
            self.parse_value(depth + 1)?;
            count += 1;
            match self.peek_struct() {
                Some(b',') => {
                    self.expect_struct(b',')?;
                }
                Some(b']') => {
                    self.expect_struct(b']')?;
                    break;
                }
                _ => return Err(self.err("expected , or ]")),
            }
        }
        self.nodes[idx] = mk_word(TAPE_ARRAY, count, 0);
        Ok(idx)
    }

    fn parse_object(&mut self, depth: usize) -> Result<usize, JsonError> {
        self.expect_struct(b'{')?;
        let idx = self.nodes.len();
        self.nodes.push(mk_word(TAPE_OBJECT, 0, 0));
        let mut pairs: Vec<(u64, usize, usize, usize)> = Vec::new();
        if self.peek_struct() == Some(b'}') {
            self.expect_struct(b'}')?;
            return Ok(idx);
        }
        loop {
            self.skip_ws();
            if self.input.get(self.pos) != Some(&b'"') {
                return Err(self.err("expected object key"));
            }
            let kidx = self.parse_key()?;
            let (fp, _) = self.key_fp(kidx);
            self.expect_struct(b':')?;
            let before = self.nodes.len();
            self.parse_value(depth + 1)?;
            let vlen = self.nodes.len() - before;
            pairs.push((fp, kidx, before, vlen));
            match self.peek_struct() {
                Some(b',') => {
                    self.expect_struct(b',')?;
                }
                Some(b'}') => {
                    self.expect_struct(b'}')?;
                    break;
                }
                _ => return Err(self.err("expected , or }")),
            }
        }
        if pairs.len() > 1 {
            let keys = &self.keys;
            let key_ends = &self.key_ends;
            let nodes = &self.nodes;
            for i in 1..pairs.len() {
                let mut j = i;
                while j > 0 {
                    let (fa, ka) = (pairs[j - 1].0, pairs[j - 1].1);
                    let (fb, kb) = (pairs[j].0, pairs[j].1);
                    let greater = if fa != fb {
                        fa > fb
                    } else {
                        key_slice(nodes, keys, key_ends, ka) > key_slice(nodes, keys, key_ends, kb)
                    };
                    if !greater {
                        break;
                    }
                    pairs.swap(j - 1, j);
                    j -= 1;
                }
            }
        }
        let keep: Vec<(u64, usize, usize, usize)> = {
            let mut out: Vec<(u64, usize, usize, usize)> = Vec::with_capacity(pairs.len());
            let keys = &self.keys;
            let key_ends = &self.key_ends;
            let nodes = &self.nodes;
            for pr in pairs {
                if let Some(last) = out.last() {
                    if last.0 == pr.0
                        && key_slice(nodes, keys, key_ends, last.1)
                            == key_slice(nodes, keys, key_ends, pr.1)
                    {
                        *out.last_mut().unwrap() = pr;
                        continue;
                    }
                }
                out.push(pr);
            }
            out
        };
        let words: Vec<u64> = keep
            .iter()
            .flat_map(|(_, k, v, l)| {
                let mut w = Vec::with_capacity(1 + l);
                w.push(self.nodes[*k]);
                w.extend_from_slice(&self.nodes[*v..*v + *l]);
                w
            })
            .collect();
        self.nodes.truncate(idx + 1);
        let count = keep.len() as u32;
        self.nodes.extend_from_slice(&words);
        self.nodes[idx] = mk_word(TAPE_OBJECT, count, 0);
        Ok(idx)
    }
}

fn key_slice<'b>(nodes: &[u64], keys: &'b [u8], key_ends: &[u32], kidx: usize) -> &'b [u8] {
    let k = tape_payload(nodes[kidx]) as usize;
    let end = key_ends[k] as usize;
    let start = if k == 0 { 0 } else { key_ends[k - 1] as usize };
    &keys[start..end]
}

pub fn parse_tape(input: &[u8]) -> Result<JsonDoc, JsonError> {
    let count = count_structurals(input);
    let mut b = Builder {
        input,
        feed: StructFeed {
            input,
            cur: StreamCursor::default(),
            pending: Vec::with_capacity(64),
            base_si: 0,
            done: false,
        },
        si: 0,
        pos: 0,
        nodes: Vec::with_capacity(count + 8),
        strings: Vec::with_capacity(input.len() / 2),
        keys: Vec::with_capacity(64),
        key_ends: Vec::with_capacity(16),
        floats: FloatCache::default(),
    };
    b.parse_value(0)?;
    b.skip_ws();
    if b.pos != input.len() || b.feed.get(b.si).is_some() {
        return Err(JsonError::new("trailing characters", b.pos));
    }
    Ok(JsonDoc {
        nodes: b.nodes,
        strings: b.strings,
        keys: b.keys,
        key_ends: b.key_ends,
    })
}

pub struct TapeRef {
    pub doc: Arc<JsonDoc>,
    pub root: u32,
    pub upgraded: *mut u8,
}

struct DocEntry {
    doc: Arc<JsonDoc>,
    refs: std::sync::atomic::AtomicUsize,
}

static TAPE_DOCS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::BTreeMap<u32, Arc<DocEntry>>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));

static NEXT_DOC_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

struct RawMap(*mut u8);

unsafe impl Send for RawMap {}

static TAPE_UPGRADED: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<(u32, u32), RawMap>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

std::thread_local! {
    static TLS_DOC: std::cell::RefCell<Option<(u32, std::sync::Weak<DocEntry>)>> =
        std::cell::RefCell::new(None);
}

pub fn tape_handle_id(handle: u64) -> u32 {
    (handle >> 32) as u32
}

pub fn tape_handle_root(handle: u64) -> u32 {
    handle as u32
}

fn tape_pack(id: u32, root: u32) -> u64 {
    ((id as u64) << 32) | root as u64
}

pub const TAPE_ID_BOUND: u32 = 0x00FF_FFFF;

fn tape_id_ok(id: u32) -> bool {
    id != 0 && id <= TAPE_ID_BOUND
}

fn tls_fast_entry(id: u32) -> Option<Arc<DocEntry>> {
    if !tape_id_ok(id) {
        return None;
    }
    TLS_DOC.with(|c| match c.borrow().as_ref() {
        Some((cid, weak)) if *cid == id => weak.upgrade(),
        _ => None,
    })
}

fn tls_remember(id: u32, entry: &Arc<DocEntry>) {
    TLS_DOC.with(|c| {
        *c.borrow_mut() = Some((id, Arc::downgrade(entry)));
    });
}

pub fn tape_materialize(doc: &Arc<JsonDoc>, root: usize) -> u64 {
    let mut docs = TAPE_DOCS.lock().unwrap_or_else(|e| e.into_inner());
    for (id, e) in docs.iter() {
        if Arc::ptr_eq(&e.doc, doc) {
            e.refs.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            tls_remember(*id, e);
            return tape_pack(*id, root as u32);
        }
    }
    let id = NEXT_DOC_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    if !tape_id_ok(id) {
        unsafe {
            super::rnx_panic(b"json doc id exhausted\0".as_ptr(), "json doc id exhausted".len());
        }
    }
    let entry = Arc::new(DocEntry {
        doc: doc.clone(),
        refs: std::sync::atomic::AtomicUsize::new(1),
    });
    docs.insert(id, entry.clone());
    tls_remember(id, &entry);
    tape_pack(id, root as u32)
}

pub fn tape_materialize_fast(doc: &Arc<JsonDoc>, root: usize) -> Option<u64> {
    TLS_DOC.with(|c| match c.borrow().as_ref() {
        Some((id, weak)) => match weak.upgrade() {
            Some(e) if Arc::ptr_eq(&e.doc, doc) => {
                e.refs.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Some(tape_pack(*id, root as u32))
            }
            _ => None,
        },
        None => None,
    })
}

pub fn tape_resolve_fast(handle: u64) -> Option<(Arc<JsonDoc>, u32)> {
    let entry = tls_fast_entry(tape_handle_id(handle))?;
    let root = tape_handle_root(handle);
    if root as usize >= entry.doc.nodes.len() {
        return None;
    }
    Some((entry.doc.clone(), root))
}

pub fn tape_resolve(handle: u64) -> Option<TapeRef> {
    let id = tape_handle_id(handle);
    if !tape_id_ok(id) {
        return None;
    }
    let docs = TAPE_DOCS.lock().unwrap_or_else(|e| e.into_inner());
    let e = docs.get(&id)?.clone();
    tls_remember(id, &e);
    let root = tape_handle_root(handle);
    if root as usize >= e.doc.nodes.len() {
        return None;
    }
    let upgraded = TAPE_UPGRADED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&(id, root))
        .map(|r| r.0)
        .unwrap_or(std::ptr::null_mut());
    Some(TapeRef { doc: e.doc.clone(), root, upgraded })
}

pub fn tape_release_fast(handle: u64) -> bool {
    let entry = match tls_fast_entry(tape_handle_id(handle)) {
        Some(e) => e,
        None => return false,
    };
    let mut cur = entry.refs.load(std::sync::atomic::Ordering::Relaxed);
    loop {
        if cur <= 1 {
            return false;
        }
        match entry.refs.compare_exchange_weak(
            cur,
            cur - 1,
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::Relaxed,
        ) {
            Ok(_) => return true,
            Err(v) => cur = v,
        }
    }
}

pub fn tape_is_handle(handle: u64) -> bool {
    let id = tape_handle_id(handle);
    if !tape_id_ok(id) {
        return false;
    }
    TAPE_DOCS.lock().unwrap_or_else(|e| e.into_inner()).contains_key(&id)
}

pub fn tape_upgrade_get(handle: u64) -> *mut u8 {
    let (id, root) = (tape_handle_id(handle), tape_handle_root(handle));
    if !tape_id_ok(id) {
        return std::ptr::null_mut();
    }
    TAPE_UPGRADED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&(id, root))
        .map(|r| r.0)
        .unwrap_or(std::ptr::null_mut())
}

pub fn tape_upgrade_set(handle: u64, map: *mut u8) {
    let (id, root) = (tape_handle_id(handle), tape_handle_root(handle));
    if !tape_id_ok(id) {
        return;
    }
    TAPE_UPGRADED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert((id, root), RawMap(map));
}

pub fn tape_release(handle: u64) -> Option<*mut u8> {
    let id = tape_handle_id(handle);
    if !tape_id_ok(id) {
        return None;
    }
    let entry = {
        let docs = TAPE_DOCS.lock().unwrap_or_else(|e| e.into_inner());
        docs.get(&id)?.clone()
    };
    if entry.refs.fetch_sub(1, std::sync::atomic::Ordering::SeqCst) > 1 {
        return None;
    }
    let mut docs = TAPE_DOCS.lock().unwrap_or_else(|e| e.into_inner());
    if entry.refs.load(std::sync::atomic::Ordering::SeqCst) != 0 {
        return None;
    }
    docs.remove(&id);
    Some(
        TAPE_UPGRADED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&(id, tape_handle_root(handle)))
            .map(|r| r.0)
            .unwrap_or(std::ptr::null_mut()),
    )
}

pub fn tape_write_doc(out: &mut impl JsonSink, doc: &JsonDoc, idx: usize, depth: usize) {
    if depth > TAPE_MAX_DEPTH {
        unsafe {
            super::rnx_panic(
                b"json max depth exceeded\0".as_ptr(),
                "json max depth exceeded".len(),
            );
        }
    }
    match tape_tag(doc.nodes[idx]) {
        TAPE_NULL => out.push_str("null"),
        TAPE_BOOL => out.push_str(if bool_at(doc, idx) { "true" } else { "false" }),
        TAPE_INT => out.push_str(&int_at(doc, idx).to_string()),
        TAPE_FLOAT => {
            let f = float_at(doc, idx);
            if !f.is_finite() {
                out.push_str("null");
            } else {
                out.push_str(&super::fmt_float(f));
            }
        }
        TAPE_STR => {
            out.push_byte(b'"');
            match std::str::from_utf8(str_at(doc, idx)) {
                Ok(s) => super::json_escape_into(out, s),
                Err(_) => out.push_str("<invalid>"),
            }
            out.push_byte(b'"');
        }
        TAPE_ARRAY => {
            out.push_byte(b'[');
            let mut first = true;
            for e in array_iter(&doc.nodes, idx) {
                if !first {
                    out.push_byte(b',');
                }
                first = false;
                tape_write_doc(out, doc, e, depth + 1);
            }
            out.push_byte(b']');
        }
        _ => {
            out.push_byte(b'{');
            let mut first = true;
            for (k, v) in object_iter(&doc.nodes, idx) {
                if !first {
                    out.push_byte(b',');
                }
                first = false;
                out.push_byte(b'"');
                match std::str::from_utf8(key_at(doc, k)) {
                    Ok(s) => super::json_escape_into(out, s),
                    Err(_) => out.push_str("<invalid>"),
                }
                out.push_byte(b'"');
                out.push_byte(b':');
                tape_write_doc(out, doc, v, depth + 1);
            }
            out.push_byte(b'}');
        }
    }
}

#[derive(Clone, Debug)]
pub enum TypedVal {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(Vec<u8>),
    Nested(JsonDoc),
}

struct Cursor<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn err(&self, msg: &str) -> JsonError {
        JsonError::new(msg, self.pos)
    }

    fn skip_ws(&mut self) {
        while self.pos < self.input.len() && is_ws(self.input[self.pos]) {
            self.pos += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_ws();
        self.input.get(self.pos).copied()
    }

    fn expect_byte(&mut self, b: u8, what: &str) -> Result<(), JsonError> {
        self.skip_ws();
        if self.input.get(self.pos) == Some(&b) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.err(what))
        }
    }

    fn hex4(&mut self) -> Result<u32, JsonError> {
        if self.pos + 4 > self.input.len() {
            return Err(self.err("bad unicode escape"));
        }
        let mut v = 0u32;
        for i in 0..4 {
            let c = self.input[self.pos + i];
            let d = match c {
                b'0'..=b'9' => (c - b'0') as u32,
                b'a'..=b'f' => (c - b'a' + 10) as u32,
                b'A'..=b'F' => (c - b'A' + 10) as u32,
                _ => return Err(self.err("bad unicode escape")),
            };
            v = v * 16 + d;
        }
        self.pos += 4;
        Ok(v)
    }

    fn raw_string(&mut self) -> Result<Vec<u8>, JsonError> {
        if self.input.get(self.pos) != Some(&b'"') {
            return Err(self.err("expected string"));
        }
        self.pos += 1;
        let mut out = Vec::new();
        loop {
            let c = *self.input.get(self.pos).ok_or_else(|| self.err("unterminated string"))?;
            if c == b'"' {
                self.pos += 1;
                return Ok(out);
            }
            if c == b'\\' {
                self.pos += 1;
                let e = *self.input.get(self.pos).ok_or_else(|| self.err("unterminated string"))?;
                match e {
                    b'"' => out.push(b'"'),
                    b'\\' => out.push(b'\\'),
                    b'/' => out.push(b'/'),
                    b'b' => out.push(0x08),
                    b'f' => out.push(0x0C),
                    b'n' => out.push(b'\n'),
                    b'r' => out.push(b'\r'),
                    b't' => out.push(b'\t'),
                    b'u' => {
                        self.pos += 1;
                        let mut cp = self.hex4()?;
                        if (0xD800..0xDC00).contains(&cp) {
                            if self.input.get(self.pos) == Some(&b'\\')
                                && self.input.get(self.pos + 1) == Some(&b'u')
                            {
                                self.pos += 2;
                                let lo = self.hex4()?;
                                if (0xDC00..0xE000).contains(&lo) {
                                    cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                                } else {
                                    return Err(self.err("bad unicode escape"));
                                }
                            } else {
                                return Err(self.err("bad unicode escape"));
                            }
                        } else if (0xDC00..0xE000).contains(&cp) {
                            return Err(self.err("bad unicode escape"));
                        }
                        match char::from_u32(cp) {
                            Some(ch) => {
                                let mut buf = [0u8; 4];
                                out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                                continue;
                            }
                            None => return Err(self.err("bad unicode escape")),
                        }
                    }
                    _ => return Err(self.err("bad escape")),
                }
                self.pos += 1;
            } else {
                if c < 0x20 {
                    return Err(self.err("unescaped control character"));
                }
                out.push(c);
                self.pos += 1;
            }
        }
    }

    fn number(&mut self) -> Result<TypedVal, JsonError> {
        let start = self.pos;
        let len = self.input.len();
        let mut p = self.pos;
        if self.input.get(p) == Some(&b'-') {
            p += 1;
        }
        let int_start = p;
        if p < len && self.input[p] == b'0' {
            p += 1;
        } else {
            while p < len && self.input[p].is_ascii_digit() {
                p += 1;
            }
            if p == int_start {
                return Err(self.err("invalid number"));
            }
        }
        let mut is_float = false;
        if p < len && self.input[p] == b'.' {
            is_float = true;
            p += 1;
            let frac = p;
            while p < len && self.input[p].is_ascii_digit() {
                p += 1;
            }
            if p == frac {
                return Err(self.err("invalid number"));
            }
        }
        if p < len && (self.input[p] == b'e' || self.input[p] == b'E') {
            is_float = true;
            p += 1;
            if p < len && (self.input[p] == b'+' || self.input[p] == b'-') {
                p += 1;
            }
            let exp = p;
            while p < len && self.input[p].is_ascii_digit() {
                p += 1;
            }
            if p == exp {
                return Err(self.err("invalid number"));
            }
        }
        let text = std::str::from_utf8(&self.input[start..p]).map_err(|_| self.err("invalid number"))?;
        self.pos = p;
        if !is_float {
            if let Ok(n) = text.parse::<i64>() {
                return Ok(TypedVal::Int(n));
            }
        }
        match text.parse::<f64>() {
            Ok(f) if f.is_finite() => Ok(TypedVal::Float(f)),
            _ => Err(self.err("invalid number")),
        }
    }

    fn literal(&mut self) -> Result<TypedVal, JsonError> {
        for (lit, val) in [
            ("true", TypedVal::Bool(true)),
            ("false", TypedVal::Bool(false)),
            ("null", TypedVal::Null),
        ] {
            let b = lit.as_bytes();
            if self.input.len() >= self.pos + b.len() && &self.input[self.pos..self.pos + b.len()] == b {
                self.pos += b.len();
                return Ok(val);
            }
        }
        Err(self.err("invalid literal"))
    }

    fn container_end(&self, start: usize) -> Result<usize, JsonError> {
        let open = self.input[start];
        let close = if open == b'{' { b'}' } else { b']' };
        let mut depth = 0usize;
        let mut i = start;
        let len = self.input.len();
        let mut in_str = false;
        let mut esc = false;
        while i < len {
            let c = self.input[i];
            if in_str {
                if esc {
                    esc = false;
                } else if c == b'\\' {
                    esc = true;
                } else if c == b'"' {
                    in_str = false;
                }
            } else if c == b'"' {
                in_str = true;
            } else if c == b'{' || c == b'[' {
                depth += 1;
            } else if c == b'}' || c == b']' {
                if depth == 0 {
                    return Err(JsonError::new("mismatched close", i));
                }
                depth -= 1;
                if depth == 0 {
                    if c != close {
                        return Err(JsonError::new("mismatched close", i));
                    }
                    return Ok(i + 1);
                }
            }
            i += 1;
        }
        Err(JsonError::new("unterminated container", start))
    }

    fn nested(&mut self) -> Result<TypedVal, JsonError> {
        let start = self.pos;
        let end = self.container_end(start)?;
        match parse_tape(&self.input[start..end]) {
            Ok(doc) => {
                self.pos = end;
                Ok(TypedVal::Nested(doc))
            }
            Err(e) => Err(JsonError::new(&e.msg, start + e.offset)),
        }
    }

    fn any_value(&mut self) -> Result<TypedVal, JsonError> {
        self.skip_ws();
        let c = *self.input.get(self.pos).ok_or_else(|| self.err("unexpected end"))?;
        match c {
            b'"' => Ok(TypedVal::Str(self.raw_string()?)),
            b'{' | b'[' => self.nested(),
            b't' | b'f' | b'n' => self.literal(),
            b'-' | b'0'..=b'9' => self.number(),
            _ => Err(self.err("unexpected character")),
        }
    }
}

pub fn parse_typed(input: &[u8], fields: &[(Vec<u8>, u8)]) -> Result<Vec<TypedVal>, JsonError> {
    let mut c = Cursor { input, pos: 0 };
    c.expect_byte(b'{', "typed decode needs a JSON object")?;
    let mut slots: Vec<TypedVal> = vec![TypedVal::Null; fields.len()];
    if c.peek() == Some(b'}') {
        c.pos += 1;
    } else {
        loop {
            c.skip_ws();
            if c.input.get(c.pos) != Some(&b'"') {
                return Err(c.err("expected object key"));
            }
            let key = c.raw_string()?;
            c.expect_byte(b':', "expected :")?;
            let slot = fields.iter().position(|(k, _)| tape_key_eq(k, &key));
            let val = c.any_value()?;
            if let Some(i) = slot {
                slots[i] = val;
            }
            c.skip_ws();
            match c.input.get(c.pos).copied() {
                Some(b',') => c.pos += 1,
                Some(b'}') => {
                    c.pos += 1;
                    break;
                }
                _ => return Err(c.err("expected , or }")),
            }
        }
    }
    c.skip_ws();
    if c.pos != input.len() {
        return Err(JsonError::new("trailing characters", c.pos));
    }
    Ok(slots)
}

pub fn split_typed_desc(desc: &str) -> Vec<(Vec<u8>, u8)> {
    let mut out = Vec::new();
    let bytes = desc.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let mut name = Vec::new();
        while i < bytes.len() {
            let b = bytes[i];
            if b == b'\\' && i + 1 < bytes.len() {
                name.push(bytes[i + 1]);
                i += 2;
                continue;
            }
            if b == b':' {
                i += 1;
                break;
            }
            name.push(b);
            i += 1;
        }
        let kind = if i < bytes.len() {
            let k = bytes[i];
            i += 1;
            k
        } else {
            b'v'
        };
        if i < bytes.len() && bytes[i] == b';' {
            i += 1;
        }
        out.push((name, kind));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root_tag(doc: &JsonDoc) -> u8 {
        tape_tag(doc.nodes[0])
    }

    #[test]
    fn scalars() {
        assert_eq!(root_tag(&parse_tape(b"null").unwrap()), TAPE_NULL);
        assert_eq!(root_tag(&parse_tape(b"true").unwrap()), TAPE_BOOL);
        assert!(bool_at(&parse_tape(b"true").unwrap(), 0));
        assert_eq!(int_at(&parse_tape(b"12345").unwrap(), 0), 12345);
        assert_eq!(int_at(&parse_tape(b"-7").unwrap(), 0), -7);
        assert_eq!(float_at(&parse_tape(b"3.1415").unwrap(), 0), 3.1415);
        assert_eq!(root_tag(&parse_tape(b"3.0").unwrap()), TAPE_FLOAT);
        let d = parse_tape(b"\"hello\"").unwrap();
        assert_eq!(str_at(&d, 0), b"hello");
    }

    #[test]
    fn key_order_mixed_lengths() {
        let d = parse_tape(b"{\"name\":\"a\",\"version\":1,\"features\":[],\"meta\":{},\"a\":0,\"abcdefghij\":1,\"abcdefghi\":2}").unwrap();
        let mut keys = Vec::new();
        for (k, _) in object_iter(&d.nodes, 0) {
            keys.push(std::str::from_utf8(key_at(&d, k)).unwrap().to_string());
        }
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
    }

    #[test]
    fn nested_sorted_last_wins() {
        let d = parse_tape(b"{\"b\":1,\"a\":2,\"b\":3}").unwrap();
        assert_eq!(object_count(&d.nodes, 0), 2);
        assert_eq!(d.key_ends.len(), 2);
        let k0 = object_key_at(&d.nodes, 0, 0);
        assert_eq!(key_at(&d, k0), b"a");
        assert_eq!(find_key(&d, 0, b"b").map(|v| int_at(&d, v)), Some(3));
        let arr = parse_tape(b"[1,[2,3],{\"x\":null}]").unwrap();
        assert_eq!(array_count(&arr.nodes, 0), 3);
    }

    #[test]
    fn escapes_unicode() {
        let d = parse_tape(b"{\"q\":\"a\\\"b\\\\c\\nd\",\"e\":\"\\u00e9\\uD83D\\uDE00\"}").unwrap();
        let q = find_key(&d, 0, b"q").unwrap();
        assert_eq!(str_at(&d, q), "a\"b\\c\nd".as_bytes());
        let e = find_key(&d, 0, b"e").unwrap();
        assert_eq!(str_at(&d, e), "é😀".as_bytes());
    }

    #[test]
    fn rejects_invalid() {
        for bad in [
            "{unquoted: 1}",
            "[",
            "{\"a\":}",
            "[1,]",
            "{\"a\" 1}",
            "[01]",
            "[1.]",
            "[.5]",
            "[1e]",
            "[+1]",
            "\"\\uD83D\"",
            "\"\x01\"",
            "nul",
            "[1 2]",
            "{\"a\":1} trailing",
            "",
            "[[]]]",
        ] {
            assert!(parse_tape(bad.as_bytes()).is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn depth_limit() {
        let deep = format!("{}{}", "[".repeat(70), "]".repeat(70));
        let e = parse_tape(deep.as_bytes()).unwrap_err();
        assert!(e.is_depth);
    }

    #[test]
    fn numbers_edge() {
        assert_eq!(int_at(&parse_tape(b"0").unwrap(), 0), 0);
        assert_eq!(root_tag(&parse_tape(b"9223372036854775807").unwrap()), TAPE_INT);
        assert_eq!(root_tag(&parse_tape(b"9223372036854775808").unwrap()), TAPE_FLOAT);
        assert_eq!(root_tag(&parse_tape(b"1e3").unwrap()), TAPE_FLOAT);
        assert_eq!(float_at(&parse_tape(b"-0.5e-2").unwrap(), 0), -0.005);
    }
}
