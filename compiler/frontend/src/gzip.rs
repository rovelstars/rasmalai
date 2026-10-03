use diagnostics::{Code, Diagnostic};

const fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            if c & 1 == 1 {
                c = 0xEDB88320 ^ (c >> 1);
            } else {
                c >>= 1;
            }
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

static CRC_TABLE: [u32; 256] = crc_table();

pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for b in data {
        crc = CRC_TABLE[((crc ^ (*b as u32)) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99,
    115, 131, 163, 195, 227, 258,
];
const LEN_EXTRA: [u8; 29] =
    [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];

const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025,
    1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12,
    12, 13, 13,
];

fn len_code(len: u16) -> (u16, u16, u8) {
    for i in 0..29 {
        let end = if i == 28 { 258 } else { LEN_BASE[i + 1] - 1 };
        if len >= LEN_BASE[i] && len <= end {
            return (257 + i as u16, len - LEN_BASE[i], LEN_EXTRA[i]);
        }
    }
    (285, 0, 0)
}

fn dist_code(dist: u16) -> (u16, u16, u8) {
    for i in 0..30 {
        let end = if i == 29 { 32768 } else { DIST_BASE[i + 1] - 1 };
        if dist >= DIST_BASE[i] && dist <= end {
            return (i as u16, dist - DIST_BASE[i], DIST_EXTRA[i]);
        }
    }
    (29, 0, 13)
}

fn fixed_litlen(code: u16) -> (u16, u8) {
    match code {
        0..=143 => (0x30 + code, 8),
        144..=255 => (0x190 + code - 144, 9),
        256..=279 => (code - 256, 7),
        _ => (0xC0 + code - 280, 8),
    }
}

struct BitWriter {
    out: Vec<u8>,
    acc: u32,
    nbits: u8,
}

impl BitWriter {
    fn new() -> BitWriter {
        BitWriter { out: Vec::new(), acc: 0, nbits: 0 }
    }

    fn put_le(&mut self, mut value: u32, mut len: u8) {
        while len > 0 {
            let take = (8 - self.nbits).min(len);
            self.acc |= (value & ((1 << take) - 1)) << self.nbits;
            value >>= take;
            len -= take;
            self.nbits += take;
            if self.nbits == 8 {
                self.out.push(self.acc as u8);
                self.acc = 0;
                self.nbits = 0;
            }
        }
    }

    fn put_msb(&mut self, code: u16, len: u8) {
        for i in (0..len).rev() {
            self.put_le(((code >> i) & 1) as u32, 1);
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.nbits > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

const HASH_BITS: usize = 15;
const HASH_MASK: usize = (1 << HASH_BITS) - 1;
const MAX_MATCH: usize = 258;
const MIN_MATCH: usize = 3;
const CHAIN_DEPTH: usize = 64;

fn deflate_fixed(data: &[u8]) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.put_le(1, 1);
    w.put_le(1, 2);
    if data.is_empty() {
        let (c, l) = fixed_litlen(256);
        w.put_msb(c, l);
        return w.finish();
    }
    let mut head = [-1i32; 1 << HASH_BITS];
    let mut prev = vec![-1i32; data.len()];
    let mut pos = 0usize;
    while pos < data.len() {
        let mut best_len = 0usize;
        let mut best_dist = 0usize;
        if pos + MIN_MATCH <= data.len() {
            let h = (((data[pos] as usize) << 10)
                ^ ((data[pos + 1] as usize) << 5)
                ^ (data[pos + 2] as usize))
                & HASH_MASK;
            let mut cand = head[h];
            let mut depth = 0;
            while cand >= 0 && depth < CHAIN_DEPTH {
                let c = cand as usize;
                let dist = pos - c;
                if dist > 32768 {
                    break;
                }
                let mut len = 0;
                while len < MAX_MATCH
                    && pos + len < data.len()
                    && data[c + len] == data[pos + len]
                {
                    len += 1;
                }
                if len > best_len {
                    best_len = len;
                    best_dist = dist;
                    if len == MAX_MATCH {
                        break;
                    }
                }
                cand = prev[c];
                depth += 1;
            }
            prev[pos] = head[h];
            head[h] = pos as i32;
        }
        if best_len >= MIN_MATCH {
            let (lc, le, ln) = len_code(best_len as u16);
            let (dc, de, dn) = dist_code(best_dist as u16);
            let (c, l) = fixed_litlen(lc);
            w.put_msb(c, l);
            w.put_le(le as u32, ln);
            w.put_msb(dc, 5);
            w.put_le(de as u32, dn);
            for k in 1..best_len {
                let q = pos + k;
                if q + MIN_MATCH <= data.len() {
                    let h = (((data[q] as usize) << 10)
                        ^ ((data[q + 1] as usize) << 5)
                        ^ (data[q + 2] as usize))
                        & HASH_MASK;
                    prev[q] = head[h];
                    head[h] = q as i32;
                }
            }
            pos += best_len;
        } else {
            let (c, l) = fixed_litlen(data[pos] as u16);
            w.put_msb(c, l);
            pos += 1;
        }
    }
    let (c, l) = fixed_litlen(256);
    w.put_msb(c, l);
    w.finish()
}

pub fn compress_gzip(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x1F, 0x8B, 8, 0, 0, 0, 0, 0, 2, 255];
    out.extend(deflate_fixed(data));
    out.extend(crc32(data).to_le_bytes());
    out.extend((data.len() as u32).to_le_bytes());
    out
}

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    acc: u32,
    nbits: u8,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> BitReader<'a> {
        BitReader { data, pos: 0, acc: 0, nbits: 0 }
    }

    fn byte_pos(&self) -> usize {
        self.pos - (self.nbits as usize) / 8
    }

    fn fill(&mut self) -> Result<(), Diagnostic> {
        while self.nbits <= 24 {
            match self.data.get(self.pos) {
                Some(b) => {
                    self.acc |= (*b as u32) << self.nbits;
                    self.nbits += 8;
                    self.pos += 1;
                }
                None => break,
            }
        }
        Ok(())
    }

    fn bit(&mut self) -> Result<u32, Diagnostic> {
        self.fill()?;
        if self.nbits == 0 {
            return Err(Diagnostic::new(Code::E108, "truncated deflate stream".to_string()));
        }
        let b = self.acc & 1;
        self.acc >>= 1;
        self.nbits -= 1;
        Ok(b)
    }

    fn bits_le(&mut self, n: u8) -> Result<u32, Diagnostic> {
        let mut v = 0u32;
        for i in 0..n {
            v |= self.bit()? << i;
        }
        Ok(v)
    }

    fn align(&mut self) {
        let skip = self.nbits % 8;
        self.acc >>= skip;
        self.nbits -= skip;
    }
}

struct Huffman {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

fn build_huffman(lengths: &[u8]) -> Result<Huffman, Diagnostic> {
    let mut counts = [0u16; 16];
    for l in lengths {
        if *l as usize >= counts.len() {
            return Err(Diagnostic::new(Code::E108, "bad huffman length".to_string()));
        }
        counts[*l as usize] += 1;
    }
    counts[0] = 0;
    let mut offs = [0u16; 16];
    let mut code = 0u16;
    for len in 1..16 {
        code = (code + counts[len - 1]) << 1;
        offs[len] = code;
    }
    let mut next = offs;
    for l in lengths.iter() {
        if *l != 0 && next[*l as usize] >= (1 << *l) {
            return Err(Diagnostic::new(Code::E108, "over-subscribed huffman".to_string()));
        }
        if *l != 0 {
            next[*l as usize] += 1;
        }
    }
    let mut symbols = Vec::with_capacity(lengths.len());
    for len in 1..16 {
        for (sym, l) in lengths.iter().enumerate() {
            if *l as usize == len {
                symbols.push(sym as u16);
            }
        }
    }
    Ok(Huffman { counts, symbols })
}

fn fixed_tables() -> (Huffman, Huffman) {
    let mut litlen = [0u8; 288];
    for v in litlen.iter_mut().take(144) {
        *v = 8;
    }
    for v in litlen.iter_mut().take(256).skip(144) {
        *v = 9;
    }
    for v in litlen.iter_mut().take(280).skip(256) {
        *v = 7;
    }
    for v in litlen.iter_mut().skip(280) {
        *v = 8;
    }
    let dist = [5u8; 30];
    (build_huffman(&litlen).expect("fixed"), build_huffman(&dist).expect("fixed"))
}

fn decode_symbol(r: &mut BitReader, h: &Huffman) -> Result<u16, Diagnostic> {
    let mut code = 0u32;
    let mut first = 0u32;
    let mut index = 0u32;
    for len in 1..16 {
        code |= r.bit()?;
        let count = h.counts[len] as u32;
        if code >= first && code - first < count {
            return Ok(h.symbols[(index + (code - first)) as usize]);
        }
        index += count;
        first = (first + count) << 1;
        code <<= 1;
    }
    Err(Diagnostic::new(Code::E108, "bad huffman code".to_string()))
}

const CL_ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

fn dynamic_tables(r: &mut BitReader) -> Result<(Huffman, Huffman), Diagnostic> {
    let hlit = r.bits_le(5)? as usize + 257;
    let hdist = r.bits_le(5)? as usize + 1;
    let hclen = r.bits_le(4)? as usize + 4;
    if hlit > 288 || hdist > 30 {
        return Err(Diagnostic::new(Code::E108, "bad dynamic header".to_string()));
    }
    let mut cl_lens = [0u8; 19];
    for i in 0..hclen {
        cl_lens[CL_ORDER[i]] = r.bits_le(3)? as u8;
    }
    let cl = build_huffman(&cl_lens)?;
    let mut lengths = vec![0u8; hlit + hdist];
    let mut i = 0;
    while i < lengths.len() {
        let sym = decode_symbol(r, &cl)?;
        match sym {
            0..=15 => {
                lengths[i] = sym as u8;
                i += 1;
            }
            16 => {
                if i == 0 {
                    return Err(Diagnostic::new(Code::E108, "bad repeat".to_string()));
                }
                let rep = r.bits_le(2)? as usize + 3;
                let v = lengths[i - 1];
                for _ in 0..rep {
                    if i >= lengths.len() {
                        return Err(Diagnostic::new(Code::E108, "bad repeat".to_string()));
                    }
                    lengths[i] = v;
                    i += 1;
                }
            }
            17 => {
                let rep = r.bits_le(3)? as usize + 3;
                for _ in 0..rep {
                    if i >= lengths.len() {
                        return Err(Diagnostic::new(Code::E108, "bad repeat".to_string()));
                    }
                    lengths[i] = 0;
                    i += 1;
                }
            }
            _ => {
                let rep = r.bits_le(7)? as usize + 11;
                for _ in 0..rep {
                    if i >= lengths.len() {
                        return Err(Diagnostic::new(Code::E108, "bad repeat".to_string()));
                    }
                    lengths[i] = 0;
                    i += 1;
                }
            }
        }
    }
    let litlen = build_huffman(&lengths[..hlit])?;
    let dist = build_huffman(&lengths[hlit..])?;
    Ok((litlen, dist))
}

fn inflate_block(
    r: &mut BitReader,
    litlen: &Huffman,
    dist: &Huffman,
    out: &mut Vec<u8>,
) -> Result<bool, Diagnostic> {
    loop {
        if out.len() > (1 << 31) {
            return Err(Diagnostic::new(Code::E108, "archive too large".to_string()));
        }
        let sym = decode_symbol(r, litlen)?;
        match sym {
            0..=255 => out.push(sym as u8),
            256 => return Ok(true),
            _ => {
                let li = (sym - 257) as usize;
                if li >= LEN_BASE.len() {
                    return Err(Diagnostic::new(Code::E108, "bad length".to_string()));
                }
                let len = LEN_BASE[li] as usize + r.bits_le(LEN_EXTRA[li])? as usize;
                let dsym = decode_symbol(r, dist)?;
                if dsym as usize >= DIST_BASE.len() {
                    return Err(Diagnostic::new(Code::E108, "bad distance".to_string()));
                }
                let d = DIST_BASE[dsym as usize] as usize + r.bits_le(DIST_EXTRA[dsym as usize])? as usize;
                if d == 0 || d > out.len() {
                    return Err(Diagnostic::new(Code::E108, "bad distance".to_string()));
                }
                for _ in 0..len {
                    let b = out[out.len() - d];
                    out.push(b);
                }
            }
        }
    }
}

pub fn decompress_gzip(data: &[u8]) -> Result<Vec<u8>, Diagnostic> {
    if data.len() < 18
        || data[0] != 0x1F
        || data[1] != 0x8B
        || data[2] != 8
    {
        return Err(Diagnostic::new(Code::E108, "not a gzip archive".to_string()));
    }
    let flg = data[3];
    let mut pos = 10usize;
    if flg & 0x04 != 0 {
        if pos + 2 > data.len() {
            return Err(Diagnostic::new(Code::E108, "bad gzip extra".to_string()));
        }
        let xlen = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2 + xlen;
    }
    if flg & 0x08 != 0 {
        while pos < data.len() && data[pos] != 0 {
            pos += 1;
        }
        pos += 1;
    }
    if flg & 0x10 != 0 {
        while pos < data.len() && data[pos] != 0 {
            pos += 1;
        }
        pos += 1;
    }
    if flg & 0x02 != 0 {
        pos += 2;
    }
    if pos > data.len() {
        return Err(Diagnostic::new(Code::E108, "bad gzip header".to_string()));
    }
    let mut r = BitReader::new(&data[pos..]);
    let mut out = Vec::new();
    let (fixed_lit, fixed_dist) = fixed_tables();
    loop {
        let last = r.bit()?;
        let btype = r.bits_le(2)?;
        match btype {
            0 => {
                r.align();
                if r.byte_pos() + 4 > r.data.len() {
                    return Err(Diagnostic::new(Code::E108, "truncated stored block".to_string()));
                }
                let base = r.byte_pos();
                let len = u16::from_le_bytes([r.data[base], r.data[base + 1]]) as usize;
                let nlen = u16::from_le_bytes([r.data[base + 2], r.data[base + 3]]);
                if len as u16 != !nlen {
                    return Err(Diagnostic::new(Code::E108, "bad stored block".to_string()));
                }
                if base + 4 + len > r.data.len() {
                    return Err(Diagnostic::new(Code::E108, "truncated stored block".to_string()));
                }
                out.extend(&r.data[base + 4..base + 4 + len]);
                r.pos = base + 4 + len;
                r.acc = 0;
                r.nbits = 0;
            }
            1 => {
                inflate_block(&mut r, &fixed_lit, &fixed_dist, &mut out)?;
            }
            2 => {
                let (litlen, dist) = dynamic_tables(&mut r)?;
                inflate_block(&mut r, &litlen, &dist, &mut out)?;
            }
            _ => return Err(Diagnostic::new(Code::E108, "bad block type".to_string())),
        }
        if last == 1 {
            break;
        }
    }
    let start = r.byte_pos();
    if start + 8 > r.data.len() {
        return Err(Diagnostic::new(Code::E108, "missing gzip trailer".to_string()));
    }
    let want_crc = u32::from_le_bytes(r.data[start..start + 4].try_into().unwrap());
    let want_len = u32::from_le_bytes(r.data[start + 4..start + 8].try_into().unwrap());
    if crc32(&out) != want_crc {
        return Err(Diagnostic::new(Code::E108, "gzip crc mismatch".to_string()));
    }
    if out.len() as u32 != want_len {
        return Err(Diagnostic::new(Code::E108, "gzip length mismatch".to_string()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_known_vector() {
        assert_eq!(crc32(b"123456789"), 0xCBF43926);
    }

    #[test]
    fn roundtrip_cases() {
        let cases: Vec<Vec<u8>> = vec![
            Vec::new(),
            b"A".to_vec(),
            b"hello world, hello world, hello world, hello world!".to_vec(),
            (0..4096u32).map(|i| (i % 251) as u8).collect(),
            vec![7u8; 3000],
        ];
        for c in &cases {
            let g = compress_gzip(c);
            assert_eq!(&g[0..10], &[0x1F, 0x8B, 8, 0, 0, 0, 0, 0, 2, 255]);
            assert_eq!(decompress_gzip(&g).unwrap(), *c);
            assert_eq!(compress_gzip(c), g);
        }
    }

    #[test]
    fn rejects_bad_magic() {
        assert!(decompress_gzip(b"not gzip at all!!!!!").is_err());
    }

    #[test]
    fn rejects_bad_crc() {
        let mut g = compress_gzip(b"hello");
        let n = g.len();
        g[n - 8] ^= 0xFF;
        assert!(decompress_gzip(&g).is_err());
    }
}
