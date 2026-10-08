use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

pub const PRODUCT_FILE: &str = "product.bin";
const MAGIC: &[u8; 8] = b"RNXDEP01";
const FORMAT_VERSION: u32 = 1;
const MAX_ENTRIES: usize = 1 << 20;
const MAX_BLOB: usize = 256 << 20;
const MAP_CAP: usize = 1 << 16;

pub const KIND_PATH: &str = "path";
pub const KIND_GIT: &str = "git";
pub const KIND_SEMVER: &str = "semver";
pub const KIND_STD: &str = "std";
pub const KIND_WORKSPACE: &str = "workspace";

pub struct DepProfile {
    pub release: bool,
    pub opt_level: u8,
    pub target: Option<String>,
    pub host: String,
    pub debug: bool,
    pub runtime_hash: String,
    pub llvm_version: String,
}

pub fn dep_toolchain_hash(profile: Option<&DepProfile>) -> String {
    let (release, opt_level, target, host, debug, runtime_hash, llvm_version) = match profile {
        Some(p) => (
            p.release,
            p.opt_level,
            p.target.as_deref(),
            p.host.as_str(),
            p.debug,
            p.runtime_hash.as_str(),
            p.llvm_version.as_str(),
        ),
        None => (false, 0, None, "unknown", false, "unknown", "unknown"),
    };
    crate::cache::fingerprint_hex(
        &crate::cache::toolchain_parts(
            release,
            opt_level,
            target,
            host,
            debug,
            env!("CARGO_PKG_VERSION"),
            runtime_hash,
            llvm_version,
        )
        .iter()
        .map(|p| p.as_slice())
        .collect::<Vec<&[u8]>>(),
    )
}

pub fn dep_cache_enabled() -> bool {
    match std::env::var("RNX_DEP_CACHE") {
        Ok(v) => {
            let v = v.trim().to_ascii_lowercase();
            !(v == "0" || v == "off" || v == "no" || v == "false")
        }
        Err(_) => true,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepFile {
    pub path: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepEdge {
    pub from: String,
    pub source: String,
    pub target: String,
    pub dep_root: Option<String>,
    pub meta: Option<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepProduct {
    pub pkg: String,
    pub kind: String,
    pub spec: String,
    pub version_key: String,
    pub toolchain: String,
    pub dep_set: String,
    pub content: String,
    pub root: String,
    pub base: String,
    pub commit: String,
    pub files: Vec<DepFile>,
    pub edges: Vec<DepEdge>,
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    let b = s.as_bytes();
    out.extend_from_slice(&(b.len() as u64).to_le_bytes());
    out.extend_from_slice(b);
}

fn put_blob(out: &mut Vec<u8>, b: &[u8]) {
    out.extend_from_slice(&(b.len() as u64).to_le_bytes());
    out.extend_from_slice(b);
}

pub fn encode_product(p: &DepProduct) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    put_str(&mut out, &p.pkg);
    put_str(&mut out, &p.kind);
    put_str(&mut out, &p.spec);
    put_str(&mut out, &p.version_key);
    put_str(&mut out, &p.toolchain);
    put_str(&mut out, &p.dep_set);
    put_str(&mut out, &p.content);
    put_str(&mut out, &p.root);
    put_str(&mut out, &p.base);
    put_str(&mut out, &p.commit);
    let mut files = p.files.clone();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    out.extend_from_slice(&(files.len() as u64).to_le_bytes());
    for f in &files {
        put_str(&mut out, &f.path);
        put_blob(&mut out, &f.bytes);
    }
    let mut edges = p.edges.clone();
    edges.sort_by(|a, b| {
        (&a.from, &a.source, &a.target, &a.dep_root, &a.meta)
            .cmp(&(&b.from, &b.source, &b.target, &b.dep_root, &b.meta))
    });
    out.extend_from_slice(&(edges.len() as u64).to_le_bytes());
    for e in &edges {
        put_str(&mut out, &e.from);
        put_str(&mut out, &e.source);
        put_str(&mut out, &e.target);
        match &e.dep_root {
            Some(r) => {
                out.push(1);
                put_str(&mut out, r);
            }
            None => out.push(0),
        }
        match &e.meta {
            Some((pkg, kind)) => {
                put_str(&mut out, pkg);
                put_str(&mut out, kind);
            }
            None => {
                put_str(&mut out, "");
                put_str(&mut out, "");
            }
        }
    }
    out
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if n > MAX_BLOB || self.pos.checked_add(n)? > self.data.len() {
            return None;
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Some(s)
    }

    fn u64_le(&mut self) -> Option<u64> {
        let raw: [u8; 8] = self.take(8)?.try_into().ok()?;
        Some(u64::from_le_bytes(raw))
    }

    fn u32_le(&mut self) -> Option<u32> {
        let raw: [u8; 4] = self.take(4)?.try_into().ok()?;
        Some(u32::from_le_bytes(raw))
    }

    fn str_val(&mut self) -> Option<String> {
        let n = self.u64_le()? as usize;
        let b = self.take(n)?;
        std::str::from_utf8(b).ok().map(|s| s.to_string())
    }

    fn blob_val(&mut self) -> Option<Vec<u8>> {
        let n = self.u64_le()? as usize;
        self.take(n).map(|b| b.to_vec())
    }
}

pub fn decode_product(data: &[u8]) -> Option<DepProduct> {
    let mut c = Cursor { data, pos: 0 };
    if c.take(8)? != MAGIC {
        return None;
    }
    if c.u32_le()? != FORMAT_VERSION {
        return None;
    }
    let pkg = c.str_val()?;
    let kind = c.str_val()?;
    let spec = c.str_val()?;
    let version_key = c.str_val()?;
    let toolchain = c.str_val()?;
    let dep_set = c.str_val()?;
    let content = c.str_val()?;
    let root = c.str_val()?;
    let base = c.str_val()?;
    let commit = c.str_val()?;
    if pkg.is_empty() || kind.is_empty() || toolchain.is_empty() || content.is_empty() {
        return None;
    }
    let nfiles = c.u64_le()? as usize;
    if nfiles > MAX_ENTRIES {
        return None;
    }
    let mut files = Vec::with_capacity(nfiles.min(1024));
    for _ in 0..nfiles {
        let path = c.str_val()?;
        if path.is_empty() {
            return None;
        }
        files.push(DepFile { path, bytes: c.blob_val()? });
    }
    let nedges = c.u64_le()? as usize;
    if nedges > MAX_ENTRIES {
        return None;
    }
    let mut edges = Vec::with_capacity(nedges.min(1024));
    for _ in 0..nedges {
        let from = c.str_val()?;
        let source = c.str_val()?;
        let target = c.str_val()?;
        if from.is_empty() || source.is_empty() || target.is_empty() {
            return None;
        }
        let flag = c.take(1)?[0];
        let dep_root = match flag {
            0 => None,
            1 => Some(c.str_val()?),
            _ => return None,
        };
        let meta_pkg = c.str_val()?;
        let meta_kind = c.str_val()?;
        let meta = if meta_pkg.is_empty() || meta_kind.is_empty() {
            None
        } else {
            Some((meta_pkg, meta_kind))
        };
        edges.push(DepEdge { from, source, target, dep_root, meta });
    }
    if c.pos != data.len() {
        return None;
    }
    Some(DepProduct {
        pkg,
        kind,
        spec,
        version_key,
        toolchain,
        dep_set,
        content,
        root,
        base,
        commit,
        files,
        edges,
    })
}

pub fn content_digest_hex(entries: &[(String, Vec<u8>)]) -> String {
    let mut sorted: Vec<(&String, &Vec<u8>)> =
        entries.iter().map(|(k, v)| (k, v)).collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    let mut hasher = Sha256::new();
    hasher.update(&(sorted.len() as u64).to_le_bytes());
    for (name, bytes) in sorted {
        let nb = name.as_bytes();
        hasher.update(&(nb.len() as u64).to_le_bytes());
        hasher.update(nb);
        hasher.update(&(bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }
    hex_of(&hasher.finalize())
}

pub fn dep_set_digest_hex(parts: &[String]) -> String {
    let mut sorted: Vec<&String> = parts.iter().collect();
    sorted.sort();
    let mut blobs: Vec<&[u8]> = Vec::with_capacity(sorted.len());
    for s in sorted {
        blobs.push(s.as_bytes());
    }
    crate::cache::fingerprint_hex(&blobs)
}

pub fn hex_bytes(bytes: &[u8]) -> String {
    hex_of(bytes)
}

fn hex_of(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from_digit((b >> 4) as u32, 16).unwrap_or('0'));
        out.push(char::from_digit((b & 0xf) as u32, 16).unwrap_or('0'));
    }
    out
}

pub fn sanitize_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '@') {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() || out == "." || out == ".." {
        out = "_".to_string();
    }
    out
}

pub fn product_path(root: &Path, pkg: &str, version_key: &str, toolchain: &str) -> PathBuf {
    crate::cache::project_deps_dir(root)
        .join(sanitize_segment(pkg))
        .join(sanitize_segment(version_key))
        .join(sanitize_segment(toolchain))
        .join(PRODUCT_FILE)
}

pub fn read_sorted(paths: &[PathBuf]) -> Option<Vec<(String, Vec<u8>)>> {
    let mut out = Vec::with_capacity(paths.len());
    for p in paths {
        let bytes = std::fs::read(p).ok()?;
        out.push((p.to_string_lossy().replace('\\', "/"), bytes));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Some(out)
}

struct Stamped<T> {
    build_gen: u64,
    val: T,
}

static GLOBAL_GEN: AtomicU64 = AtomicU64::new(1);
static MEMO: Mutex<
    BTreeMap<(PathBuf, String), Stamped<(PathBuf, Option<PathBuf>, Option<(String, String)>)>>,
> = Mutex::new(BTreeMap::new());
static BYTES: Mutex<BTreeMap<PathBuf, Stamped<String>>> = Mutex::new(BTreeMap::new());

thread_local! {
    static BUILD_GEN: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

static HITS: AtomicU64 = AtomicU64::new(0);
static MEMO_HITS: AtomicU64 = AtomicU64::new(0);
static POPULATED: AtomicU64 = AtomicU64::new(0);

pub fn stats() -> (u64, u64, u64) {
    (
        HITS.load(Ordering::Relaxed),
        MEMO_HITS.load(Ordering::Relaxed),
        POPULATED.load(Ordering::Relaxed),
    )
}

pub fn begin_build() -> Option<u64> {
    if !dep_cache_enabled() {
        return None;
    }
    let build_gen = GLOBAL_GEN.fetch_add(1, Ordering::SeqCst);
    BUILD_GEN.with(|g| g.set(build_gen));
    Some(build_gen)
}

fn active_gen() -> Option<u64> {
    if !dep_cache_enabled() {
        return None;
    }
    let build_gen = BUILD_GEN.with(|g| g.get());
    if build_gen == 0 { None } else { Some(build_gen) }
}

fn evict_stale<T>(map: &mut BTreeMap<PathBuf, Stamped<T>>, build_gen: u64)
where
    T: Clone,
{
    if map.len() <= MAP_CAP {
        return;
    }
    map.retain(|_, v| v.build_gen == build_gen);
}

fn evict_stale_memo(
    map: &mut BTreeMap<(PathBuf, String), Stamped<(PathBuf, Option<PathBuf>, Option<(String, String)>)>>,
    build_gen: u64,
) {
    if map.len() <= MAP_CAP {
        return;
    }
    map.retain(|_, v| v.build_gen == build_gen);
}

pub fn memo_lookup(
    from: &Path,
    source: &str,
) -> Option<(PathBuf, Option<PathBuf>, Option<(String, String)>)> {
    let build_gen = active_gen()?;
    let key = (from.to_path_buf(), source.to_string());
    let hit = MEMO
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&key)
        .filter(|s| s.build_gen == build_gen)
        .map(|s| s.val.clone());
    if hit.is_some() {
        MEMO_HITS.fetch_add(1, Ordering::Relaxed);
    }
    hit
}

pub fn memo_install(
    build_gen: u64,
    edges: &[(PathBuf, String, PathBuf, Option<PathBuf>, Option<(String, String)>)],
) {
    if edges.is_empty() {
        return;
    }
    let mut map = MEMO.lock().unwrap_or_else(|e| e.into_inner());
    evict_stale_memo(&mut map, build_gen);
    for (from, source, target, dep_root, meta) in edges {
        map.insert(
            (from.clone(), source.clone()),
            Stamped { build_gen, val: (target.clone(), dep_root.clone(), meta.clone()) },
        );
    }
}

pub fn bytes_get(path: &Path) -> Option<String> {
    let build_gen = active_gen()?;
    BYTES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(path)
        .filter(|s| s.build_gen == build_gen)
        .map(|s| s.val.clone())
}

pub fn bytes_install(build_gen: u64, files: &[(PathBuf, String)]) {
    if files.is_empty() {
        return;
    }
    let mut map = BYTES.lock().unwrap_or_else(|e| e.into_inner());
    evict_stale(&mut map, build_gen);
    for (path, src) in files {
        map.insert(path.clone(), Stamped { build_gen, val: src.clone() });
    }
}

pub fn note_hit() {
    HITS.fetch_add(1, Ordering::Relaxed);
}

pub fn note_populated() {
    POPULATED.fetch_add(1, Ordering::Relaxed);
}

#[cfg(test)]
pub fn reset_stats() {
    HITS.store(0, Ordering::Relaxed);
    MEMO_HITS.store(0, Ordering::Relaxed);
    POPULATED.store(0, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> DepProduct {
        DepProduct {
            pkg: "@acme/widget".to_string(),
            kind: KIND_SEMVER.to_string(),
            spec: "semver:^1.0.0".to_string(),
            version_key: "1.2.0".to_string(),
            toolchain: "abc123".to_string(),
            dep_set: "depsed".to_string(),
            content: "contenthex".to_string(),
            root: "/tmp/x".to_string(),
            base: "https://example.com".to_string(),
            commit: String::new(),
            files: vec![
                DepFile { path: "/tmp/x/src/a.rnx".to_string(), bytes: b"a".to_vec() },
                DepFile { path: "/tmp/x/src/b.rnx".to_string(), bytes: b"b".to_vec() },
            ],
            edges: vec![DepEdge {
                from: "/tmp/x/src/a.rnx".to_string(),
                source: "./b".to_string(),
                target: "/tmp/x/src/b.rnx".to_string(),
                dep_root: Some("/tmp/x".to_string()),
                meta: Some(("@acme/widget".to_string(), KIND_SEMVER.to_string())),
            }],
        }
    }

    #[test]
    fn product_roundtrip_is_deterministic() {
        let bytes = encode_product(&sample());
        let back = decode_product(&bytes).expect("decode");
        assert_eq!(back, sample());
        assert_eq!(encode_product(&back), bytes);
    }

    #[test]
    fn product_rejects_corrupt_bytes() {
        let bytes = encode_product(&sample());
        assert!(decode_product(&[]).is_none());
        assert!(decode_product(b"garbage garbage garbage!!").is_none());
        for cut in [0, 8, 9, 12, bytes.len() / 2, bytes.len() - 1] {
            assert!(decode_product(&bytes[..cut]).is_none(), "cut {cut}");
        }
        let mut bad = bytes.clone();
        bad.extend_from_slice(b"trailing");
        assert!(decode_product(&bad).is_none());
        let mut flipped = bytes.clone();
        let mid = flipped.len() / 2;
        flipped[mid] ^= 0xff;
        let flipped_ok = decode_product(&flipped).is_some();
        if flipped_ok {
            assert_ne!(encode_product(&decode_product(&flipped).unwrap()), bytes);
        }
    }

    #[test]
    fn product_rejects_bad_magic_and_version() {
        let mut bytes = encode_product(&sample());
        bytes[0] = b'X';
        assert!(decode_product(&bytes).is_none());
        let mut good = encode_product(&sample());
        good[8] = 0x7f;
        assert!(decode_product(&good).is_none());
    }

    #[test]
    fn digests_cover_content_and_order() {
        let a = vec![
            ("b".to_string(), b"2".to_vec()),
            ("a".to_string(), b"1".to_vec()),
        ];
        let b = vec![
            ("a".to_string(), b"1".to_vec()),
            ("b".to_string(), b"2".to_vec()),
        ];
        assert_eq!(content_digest_hex(&a), content_digest_hex(&b));
        let mut c = b.clone();
        c[0].1 = b"changed".to_vec();
        assert_ne!(content_digest_hex(&b), content_digest_hex(&c));
        assert_eq!(dep_set_digest_hex(&["x".to_string(), "y".to_string()]), dep_set_digest_hex(&["y".to_string(), "x".to_string()]));
        assert_ne!(dep_set_digest_hex(&["x".to_string()]), dep_set_digest_hex(&["y".to_string()]));
    }

    #[test]
    fn sanitize_never_escapes() {
        assert_eq!(sanitize_segment("@acme/widget"), "@acme_widget");
        assert_eq!(sanitize_segment("../../etc"), ".._.._etc");
        assert_eq!(sanitize_segment(""), "_");
        assert_eq!(sanitize_segment("1.2.0"), "1.2.0");
        let p = product_path(Path::new("/root"), "@acme/widget", "1.2.0", "tool");
        assert_eq!(
            p,
            Path::new("/root")
                .join(".rnx-cache")
                .join("deps")
                .join("@acme_widget")
                .join("1.2.0")
                .join("tool")
                .join("product.bin")
        );
    }

    #[test]
    fn toolchain_hash_tracks_parts() {
        let (_guard, _cache) = crate::fetch::testkit::isolate_cache("dephash");
        let none = dep_toolchain_hash(None);
        assert_eq!(none.len(), 64);
        assert_eq!(dep_toolchain_hash(None), none);
        let prof = DepProfile {
            release: true,
            opt_level: 2,
            target: None,
            host: "x".to_string(),
            debug: false,
            runtime_hash: "rt".to_string(),
            llvm_version: "llvm22".to_string(),
        };
        assert_ne!(dep_toolchain_hash(Some(&prof)), none);
    }
}
