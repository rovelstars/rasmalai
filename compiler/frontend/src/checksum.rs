use diagnostics::{Code, Diagnostic};
use std::path::Path;

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

pub struct Sha256 {
    state: [u32; 8],
    block: [u8; 64],
    used: usize,
    total: u64,
}

impl Default for Sha256 {
    fn default() -> Sha256 {
        Sha256::new()
    }
}

impl Sha256 {
    pub fn new() -> Sha256 {
        Sha256 {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            block: [0; 64],
            used: 0,
            total: 0,
        }
    }

    pub fn update(&mut self, mut bytes: &[u8]) {
        self.total += bytes.len() as u64;
        while !bytes.is_empty() {
            let room = 64 - self.used;
            let take = room.min(bytes.len());
            self.block[self.used..self.used + take].copy_from_slice(&bytes[..take]);
            self.used += take;
            bytes = &bytes[take..];
            if self.used == 64 {
                compress(&mut self.state, &self.block);
                self.used = 0;
            }
        }
    }

    pub fn finish(mut self) -> [u8; 32] {
        let bit_len = self.total.wrapping_mul(8);
        self.update(&[0x80]);
        while self.used != 56 {
            self.update(&[0x00]);
        }
        self.update(&bit_len.to_be_bytes());
        let mut out = [0u8; 32];
        for (i, w) in self.state.iter().enumerate() {
            out[4 * i..4 * i + 4].copy_from_slice(&w.to_be_bytes());
        }
        out
    }

    pub fn hexdigest(bytes: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(bytes);
        hex_of(&h.finish())
    }
}

fn compress(state: &mut [u32; 8], block: &[u8; 64]) {
    let mut w = [0u32; 64];
    for i in 0..16 {
        w[i] = u32::from_be_bytes([block[4 * i], block[4 * i + 1], block[4 * i + 2], block[4 * i + 3]]);
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
    }
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for i in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ ((!e) & g);
        let t1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let t2 = s0.wrapping_add(maj);
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }
    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
    state[4] = state[4].wrapping_add(e);
    state[5] = state[5].wrapping_add(f);
    state[6] = state[6].wrapping_add(g);
    state[7] = state[7].wrapping_add(h);
}

pub fn hex_of(digest: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push(char::from_digit((b >> 4) as u32, 16).unwrap_or('0'));
        out.push(char::from_digit((b & 0x0f) as u32, 16).unwrap_or('0'));
    }
    out
}

fn rel_unix(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let mut parts: Vec<String> = Vec::new();
    for c in rel.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::Normal(s) => parts.push(s.to_string_lossy().into_owned()),
            _ => return None,
        }
    }
    Some(parts.join("/"))
}

fn excluded(name: &str) -> bool {
    name == ".git" || name == "target" || name == "tests" || name.starts_with('.')
}

pub fn compute_package_checksum(dir: &Path) -> Result<String, Diagnostic> {
    let root = std::fs::canonicalize(dir)
        .map_err(|e| Diagnostic::new(Code::E108, format!("cannot read `{}`: {e}", dir.display())))?;
    let mut files: Vec<String> = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(d) = stack.pop() {
        let entries = std::fs::read_dir(&d)
            .map_err(|e| Diagnostic::new(Code::E108, format!("cannot read `{}`: {e}", d.display())))?;
        for entry in entries {
            let entry = entry.map_err(|e| {
                Diagnostic::new(Code::E108, format!("cannot read `{}`: {e}", d.display()))
            })?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let ft = entry.file_type().map_err(|e| {
                Diagnostic::new(Code::E108, format!("cannot read `{}`: {e}", path.display()))
            })?;
            if ft.is_dir() {
                if !excluded(&name) {
                    stack.push(path);
                }
            } else if ft.is_file() {
                let is_manifest = name == crate::project::MANIFEST_FILE;
                let is_source = name.ends_with(".rnx");
                if is_manifest || is_source {
                    match rel_unix(&root, &path) {
                        Some(rel) => files.push(rel),
                        None => continue,
                    }
                }
            }
        }
    }
    files.sort();
    let mut h = Sha256::new();
    for rel in &files {
        h.update(rel.as_bytes());
        h.update(&[0x00]);
        let bytes = std::fs::read(root.join(rel)).map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot read `{rel}`: {e}"))
        })?;
        h.update(&bytes);
    }
    Ok(hex_of(&h.finish()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_known_vectors() {
        assert_eq!(
            Sha256::hexdigest(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            Sha256::hexdigest(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            Sha256::hexdigest(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        let mut h = Sha256::new();
        h.update(b"a");
        h.update(b"bc");
        assert_eq!(
            hex_of(&h.finish()),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn package_checksum_is_deterministic_and_sensitive() {
        let base = std::env::temp_dir().join(format!("rnx-sum-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("src")).unwrap();
        std::fs::write(base.join("Project.config"), "[project]\nname = \"s\"\nversion = \"1\"\n").unwrap();
        std::fs::write(base.join("src").join("main.rnx"), "fn Main(): Int { return 1; }\n").unwrap();
        let first = compute_package_checksum(&base).unwrap();
        assert_eq!(first.len(), 64);
        assert_eq!(compute_package_checksum(&base).unwrap(), first);
        std::fs::write(base.join("src").join("main.rnx"), "fn Main(): Int { return 2; }\n").unwrap();
        assert_ne!(compute_package_checksum(&base).unwrap(), first);
        let _ = std::fs::remove_dir_all(&base);
    }
}
