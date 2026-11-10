use crate::pack::{Pack, PackError, is_digest, pack_files};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

pub const STORE_DIR: &str = "sysroots";
pub const CURRENT_FILE: &str = "current";
pub const TMP_DIR: &str = "tmp";
const STALE_TMP: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub triple: String,
    pub digest: String,
    pub tier: String,
    pub current: bool,
    pub path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct Store {
    root: PathBuf,
}

pub fn cache_root() -> PathBuf {
    if let Ok(dir) = std::env::var("RNX_CACHE_HOME")
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    #[cfg(windows)]
    {
        if let Ok(local) = std::env::var("LOCALAPPDATA")
            && !local.is_empty()
        {
            return PathBuf::from(local).join("rnx");
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(xdg) = std::env::var("XDG_CACHE_HOME")
            && !xdg.is_empty()
        {
            return PathBuf::from(xdg).join("rnx");
        }
    }
    home_dir().join(".cache").join("rnx")
}

fn home_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Ok(profile) = std::env::var("USERPROFILE")
            && !profile.is_empty()
        {
            return PathBuf::from(profile);
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(home) = std::env::var("HOME")
            && !home.is_empty()
        {
            return PathBuf::from(home);
        }
    }
    PathBuf::from(".")
}

pub fn offline_flag() -> bool {
    std::env::var("RNX_OFFLINE")
        .map(|value| !value.is_empty() && value != "0")
        .unwrap_or(false)
}

pub fn fetch_disabled() -> bool {
    if offline_flag() {
        return true;
    }
    match std::env::var("CI") {
        Ok(value) => {
            let value = value.trim();
            !(value.is_empty() || value == "0" || value.eq_ignore_ascii_case("false"))
        }
        Err(_) => false,
    }
}

impl Store {
    pub fn default_store() -> Store {
        Store::new(cache_root().join(STORE_DIR))
    }

    pub fn at(cache_home: &Path) -> Store {
        Store::new(cache_home.join(STORE_DIR))
    }

    pub fn new(root: PathBuf) -> Store {
        Store { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn tmp_root(&self) -> PathBuf {
        self.root.join(TMP_DIR)
    }

    pub fn staging_dir(&self) -> Result<PathBuf, PackError> {
        let tmp = self.tmp_root();
        std::fs::create_dir_all(&tmp).map_err(|e| {
            PackError::new(
                &self.root,
                format!("cannot create `{}`: {e}", tmp.display()),
            )
        })?;
        let dir = tmp.join(unique_name());
        std::fs::create_dir(&dir).map_err(|e| {
            PackError::new(
                &self.root,
                format!("cannot create `{}`: {e}", dir.display()),
            )
        })?;
        Ok(dir)
    }

    pub fn install_from_dir(
        &self,
        source: &Path,
        expected: Option<&str>,
    ) -> Result<Installed, PackError> {
        let pack = Pack::load(source)?;
        let digest = pack.manifest.integrity.digest.clone();
        if let Some(want) = expected
            && want != digest
        {
            return Err(PackError::new(
                source,
                format!("digest mismatch: expected {want}, pack is {digest}"),
            ));
        }
        let triple = pack.target.triple.clone();
        let final_dir = self.root.join(&triple).join(&digest);
        if final_dir.is_dir() && verified(&final_dir, &digest) {
            self.write_current(&triple, &digest)?;
            return Ok(installed(&triple, &digest, true, final_dir));
        }
        if final_dir.is_dir() {
            std::fs::remove_dir_all(&final_dir).map_err(|e| {
                PackError::new(
                    &self.root,
                    format!("cannot replace corrupt `{}`: {e}", final_dir.display()),
                )
            })?;
        }
        let stage = self.staging_dir()?;
        let copied = copy_tree(source, &stage);
        if let Err(e) = copied {
            let _ = std::fs::remove_dir_all(&stage);
            return Err(e);
        }
        if let Err(e) = Pack::load(&stage) {
            let _ = std::fs::remove_dir_all(&stage);
            return Err(e);
        }
        let parent = self.root.join(&triple);
        std::fs::create_dir_all(&parent).map_err(|e| {
            PackError::new(
                &self.root,
                format!("cannot create `{}`: {e}", parent.display()),
            )
        })?;
        let staged = parent.join(format!(".stage-{}", unique_name()));
        if let Err(e) = link_tree(&stage, &staged) {
            let _ = std::fs::remove_dir_all(&stage);
            let _ = std::fs::remove_dir_all(&staged);
            return Err(e);
        }
        let renamed = std::fs::rename(&staged, &final_dir);
        if let Err(e) = renamed {
            let _ = std::fs::remove_dir_all(&staged);
            let _ = std::fs::remove_dir_all(&stage);
            if final_dir.is_dir() && verified(&final_dir, &digest) {
                self.write_current(&triple, &digest)?;
                return Ok(installed(&triple, &digest, true, final_dir));
            }
            return Err(PackError::new(
                &self.root,
                format!("cannot install into `{}`: {e}", final_dir.display()),
            ));
        }
        let _ = std::fs::remove_dir_all(&stage);
        self.write_current(&triple, &digest)?;
        Ok(installed(&triple, &digest, true, final_dir))
    }

    pub fn list(&self) -> Vec<Installed> {
        let mut out = Vec::new();
        let entries = match std::fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(_) => return out,
        };
        let mut triples: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_dir()
                    && path.file_name().and_then(|n| n.to_str()) != Some(TMP_DIR)
                    && !path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or_default()
                        .starts_with('.')
            })
            .collect();
        triples.sort();
        for triple_dir in triples {
            let Some(triple) = triple_dir.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let current = read_current(&triple_dir);
            let entries = match std::fs::read_dir(&triple_dir) {
                Ok(entries) => entries,
                Err(_) => continue,
            };
            let mut digests: Vec<String> = entries
                .flatten()
                .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
                .filter(|name| is_digest(name))
                .collect();
            digests.sort();
            for digest in digests {
                let path = triple_dir.join(&digest);
                if !path.is_dir() {
                    continue;
                }
                let is_current = current.as_deref() == Some(digest.as_str());
                out.push(installed(triple, &digest, is_current, path));
            }
        }
        out.sort_by(|a, b| a.triple.cmp(&b.triple).then(a.digest.cmp(&b.digest)));
        out
    }

    pub fn current(&self, triple: &str) -> Option<Installed> {
        let dir = self.triple_dir(triple)?;
        let digest = read_current(&dir)?;
        let path = dir.join(&digest);
        path.is_dir()
            .then(|| installed(triple, &digest, true, path))
    }

    pub fn current_digest(&self, triple: &str) -> Option<String> {
        self.current(triple).map(|item| item.digest)
    }

    pub fn path_for(&self, triple: &str, digest: &str) -> Option<PathBuf> {
        let path = self.triple_dir(triple)?.join(digest);
        path.is_dir().then_some(path)
    }

    pub fn verify(&self, triple: &str) -> Result<Installed, PackError> {
        let item = self.current(triple).ok_or_else(|| {
            PackError::new(
                &self.root,
                format!("no target pack installed for `{triple}`"),
            )
        })?;
        self.verify_digest(triple, &item.digest)
    }

    pub fn verify_digest(&self, triple: &str, digest: &str) -> Result<Installed, PackError> {
        if !is_digest(digest) {
            return Err(PackError::new(
                &self.root,
                format!("`{digest}` is not a `sha256:<hex>` digest"),
            ));
        }
        let path = self.path_for(triple, digest).ok_or_else(|| {
            PackError::new(
                &self.root,
                format!("target pack `{triple}` at {digest} is not installed"),
            )
        })?;
        let pack = Pack::load(&path)?;
        if pack.manifest.integrity.digest != digest {
            return Err(PackError::new(
                &path,
                format!(
                    "installed digest is {}, but the store path says {digest}",
                    pack.manifest.integrity.digest
                ),
            ));
        }
        let is_current = self.current_digest(triple).as_deref() == Some(digest);
        Ok(installed(triple, digest, is_current, path))
    }

    pub fn remove(&self, triple: &str) -> Result<usize, PackError> {
        if !safe_triple(triple) {
            return Err(PackError::new(
                &self.root,
                format!("`{triple}` is not a target triple"),
            ));
        }
        let dir = self.root.join(triple);
        if !dir.is_dir() {
            return Err(PackError::new(
                &self.root,
                format!("no target pack installed for `{triple}`"),
            ));
        }
        let count = std::fs::read_dir(&dir)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|entry| entry.file_name().to_str().map(is_digest).unwrap_or(false))
                    .count()
            })
            .unwrap_or(0);
        std::fs::remove_dir_all(&dir).map_err(|e| {
            PackError::new(
                &self.root,
                format!("cannot remove `{}`: {e}", dir.display()),
            )
        })?;
        Ok(count)
    }

    pub fn prune(&self, keep: Option<usize>) -> Result<usize, PackError> {
        let keep = keep.unwrap_or(1).max(1);
        self.clear_stale_tmp();
        let entries = match std::fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(_) => return Ok(0),
        };
        let mut removed = 0;
        let mut triples: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_dir()
                    && path.file_name().and_then(|n| n.to_str()) != Some(TMP_DIR)
                    && !path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or_default()
                        .starts_with('.')
            })
            .collect();
        triples.sort();
        for triple_dir in triples {
            let current = read_current(&triple_dir);
            let entries = match std::fs::read_dir(&triple_dir) {
                Ok(entries) => entries,
                Err(_) => continue,
            };
            let mut digests: Vec<(String, SystemTime)> = entries
                .flatten()
                .filter_map(|entry| {
                    let name = entry.file_name().to_str().map(str::to_string)?;
                    if !is_digest(&name) || !entry.path().is_dir() {
                        return None;
                    }
                    let modified = entry
                        .metadata()
                        .and_then(|meta| meta.modified())
                        .unwrap_or(SystemTime::UNIX_EPOCH);
                    Some((name, modified))
                })
                .collect();
            digests.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            let mut retained: BTreeSet<String> = BTreeSet::new();
            if let Some(current) = &current
                && digests.iter().any(|(name, _)| name == current)
            {
                retained.insert(current.clone());
            }
            for (digest, _) in &digests {
                if retained.contains(digest) {
                    continue;
                }
                if retained.len() < keep {
                    retained.insert(digest.clone());
                    continue;
                }
                let path = triple_dir.join(digest);
                std::fs::remove_dir_all(&path).map_err(|e| {
                    PackError::new(
                        &self.root,
                        format!("cannot remove `{}`: {e}", path.display()),
                    )
                })?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    fn triple_dir(&self, triple: &str) -> Option<PathBuf> {
        if !safe_triple(triple) {
            return None;
        }
        Some(self.root.join(triple))
    }

    fn write_current(&self, triple: &str, digest: &str) -> Result<(), PackError> {
        let dir = self.root.join(triple);
        std::fs::create_dir_all(&dir).map_err(|e| {
            PackError::new(
                &self.root,
                format!("cannot create `{}`: {e}", dir.display()),
            )
        })?;
        let temp = dir.join(format!(".current-{}", unique_name()));
        std::fs::write(&temp, format!("{digest}\n"))
            .map_err(|e| PackError::new(&dir, format!("cannot write `{}`: {e}", temp.display())))?;
        std::fs::rename(&temp, dir.join(CURRENT_FILE)).map_err(|e| {
            let _ = std::fs::remove_file(&temp);
            PackError::new(&dir, format!("cannot update `{CURRENT_FILE}`: {e}"))
        })
    }

    fn clear_stale_tmp(&self) {
        let tmp = self.tmp_root();
        let Ok(entries) = std::fs::read_dir(&tmp) else {
            return;
        };
        let now = SystemTime::now();
        for entry in entries.flatten() {
            let old = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|modified| now.duration_since(modified).ok())
                .is_some_and(|age| age > STALE_TMP);
            if old {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
}

fn installed(triple: &str, digest: &str, current: bool, path: PathBuf) -> Installed {
    let tier = crate::pack::read_manifest(&path)
        .map(|manifest| manifest.tier.to_string())
        .unwrap_or_else(|_| "?".to_string());
    Installed {
        triple: triple.to_string(),
        digest: digest.to_string(),
        tier,
        current,
        path,
    }
}

fn verified(dir: &Path, digest: &str) -> bool {
    Pack::load(dir)
        .map(|pack| pack.manifest.integrity.digest == digest)
        .unwrap_or(false)
}

fn read_current(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(CURRENT_FILE)).ok()?;
    let digest = text.trim();
    is_digest(digest).then(|| digest.to_string())
}

fn safe_triple(triple: &str) -> bool {
    !triple.is_empty()
        && triple != "."
        && triple != ".."
        && triple
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn copy_tree(source: &Path, dest: &Path) -> Result<(), PackError> {
    for rel in pack_files(source)? {
        let from = source.join(&rel);
        let to = dest.join(&rel);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                PackError::new(source, format!("cannot create `{}`: {e}", parent.display()))
            })?;
        }
        std::fs::copy(&from, &to)
            .map_err(|e| PackError::at(source, &rel, format!("cannot copy into the store: {e}")))?;
    }
    Ok(())
}

fn link_tree(source: &Path, dest: &Path) -> Result<(), PackError> {
    for rel in pack_files(source)? {
        let from = source.join(&rel);
        let to = dest.join(&rel);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                PackError::new(source, format!("cannot create `{}`: {e}", parent.display()))
            })?;
        }
        if std::fs::hard_link(&from, &to).is_err() {
            std::fs::copy(&from, &to).map_err(|e| {
                PackError::at(source, &rel, format!("cannot copy into the store: {e}"))
            })?;
        }
    }
    Ok(())
}

fn unique_name() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!(
        "{}-{}-{nanos}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::MANIFEST_FILE;
    use crate::target::Target;
    use std::fs;

    struct Scratch {
        dir: PathBuf,
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    fn scratch(tag: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!(
            "rnx-store-{tag}-{}-{}",
            std::process::id(),
            unique_name()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    fn manifest_json(triple: &str) -> String {
        let target = Target::parse(triple).unwrap();
        let capabilities: Vec<String> = target
            .capabilities
            .iter()
            .map(|c| format!("\"{}\"", c.as_str()))
            .collect();
        let lib = format!("lib/rustlib/{triple}/lib");
        format!(
            "{{\"version\": 2, \"target\": \"{triple}\", \"arch\": \"{}\", \"os\": \"{}\", \"env\": \"{}\", \"object\": \"{}\", \"libc\": \"{}\", \"pointer_width\": {}, \"endian\": \"{}\", \"tier\": {}, \"capabilities\": [{}], \"cpu\": {{\"model\": \"generic\", \"features\": \"\"}}, \"loader\": null, \"kernel_min\": null, \"glibc_baseline\": null, \"prelude\": \"@std/prelude\", \"link\": {{\"search\": [\"{lib}\"], \"crt\": {{\"static\": [\"crt1.o\"]}}, \"crt_end\": {{}}, \"libs\": {{}}, \"builtins\": null, \"runtime\": \"libruntime_native.a\"}}, \"runtime\": {{\"abi\": 1, \"file\": \"libruntime_native.a\", \"sha256\": null}}, \"integrity\": {{\"algorithm\": \"sha256\", \"digest\": \"sha256:{}\"}}}}",
            target.arch.as_str(),
            target.os.as_str(),
            target.env.as_str(),
            target.object.as_str(),
            target.libc.as_str(),
            target.pointer_width,
            target.endian.as_str(),
            match target.tier.as_str() {
                "1" => 1,
                "2" => 2,
                _ => 3,
            },
            capabilities.join(", "),
            "0".repeat(64),
        )
    }

    fn write_pack(dir: &Path, triple: &str, runtime: &[u8]) {
        let lib = dir.join("lib").join("rustlib").join(triple).join("lib");
        fs::create_dir_all(&lib).unwrap();
        fs::write(lib.join("crt1.o"), b"crt1").unwrap();
        fs::write(lib.join("libruntime_native.a"), runtime).unwrap();
        fs::write(dir.join(MANIFEST_FILE), manifest_json(triple)).unwrap();
        let digest = Pack::digest(dir).unwrap();
        let body = manifest_json(triple).replace(&"0".repeat(64), &digest[7..]);
        fs::write(dir.join(MANIFEST_FILE), body).unwrap();
    }

    fn pack(tag: &str, triple: &str, runtime: &[u8]) -> Scratch {
        let dir = scratch(tag);
        write_pack(&dir.dir, triple, runtime);
        dir
    }

    #[test]
    fn install_list_current_verify_remove() {
        let home = scratch("home");
        let store = Store::at(&home.dir);
        let fixture = pack("one", "x86_64-unknown-linux-musl", b"runtime-1");
        let installed = store.install_from_dir(&fixture.dir, None).unwrap();
        assert!(crate::pack::is_digest(&installed.digest));
        assert_eq!(installed.triple, "x86_64-unknown-linux-musl");
        assert_eq!(installed.tier, "2");
        assert!(installed.current);

        let listed = store.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].digest, installed.digest);
        assert!(listed[0].current);
        assert_eq!(
            store.current_digest("x86_64-unknown-linux-musl"),
            Some(installed.digest.clone())
        );
        assert_eq!(
            store.path_for("x86_64-unknown-linux-musl", &installed.digest),
            Some(installed.path.clone())
        );
        store.verify("x86_64-unknown-linux-musl").unwrap();
        assert_eq!(store.remove("x86_64-unknown-linux-musl").unwrap(), 1);
        assert!(store.list().is_empty());
        assert!(store.verify("x86_64-unknown-linux-musl").is_err());
    }

    #[test]
    fn install_is_deterministic_and_reuses_the_same_digest() {
        let home = scratch("home");
        let store = Store::at(&home.dir);
        let fixture = pack("det", "aarch64-unknown-linux-musl", b"runtime");
        let first = store.install_from_dir(&fixture.dir, None).unwrap();
        let second = store.install_from_dir(&fixture.dir, None).unwrap();
        assert_eq!(first.digest, second.digest);
        assert_eq!(first.path, second.path);
        assert_eq!(store.list().len(), 1);

        let other_home = scratch("other");
        let other = Store::at(&other_home.dir);
        let third = other.install_from_dir(&fixture.dir, None).unwrap();
        assert_eq!(first.digest, third.digest);
    }

    #[test]
    fn corrupt_store_entry_fails_verify() {
        let home = scratch("home");
        let store = Store::at(&home.dir);
        let fixture = pack("corrupt", "x86_64-unknown-linux-musl", b"runtime");
        let installed = store.install_from_dir(&fixture.dir, None).unwrap();
        fs::write(
            installed
                .path
                .join("lib/rustlib/x86_64-unknown-linux-musl/lib/crt1.o"),
            b"bad",
        )
        .unwrap();
        let err = store.verify("x86_64-unknown-linux-musl").unwrap_err();
        assert_eq!(err.code(), diagnostics::Code::E124);
        assert!(
            err.to_string().contains("integrity digest mismatch"),
            "{err}"
        );
    }

    #[test]
    fn expected_digest_must_match() {
        let home = scratch("home");
        let store = Store::at(&home.dir);
        let fixture = pack("expect", "x86_64-unknown-linux-musl", b"runtime");
        let err = store
            .install_from_dir(&fixture.dir, Some(&format!("sha256:{}", "0".repeat(64))))
            .unwrap_err();
        assert!(err.to_string().contains("digest mismatch"), "{err}");
        assert!(store.list().is_empty());
    }

    #[test]
    fn prune_keeps_current_and_drops_old_digests() {
        let home = scratch("home");
        let store = Store::at(&home.dir);
        let first = pack("first", "x86_64-unknown-linux-musl", b"one");
        let second = pack("second", "x86_64-unknown-linux-musl", b"two");
        let a = store.install_from_dir(&first.dir, None).unwrap();
        let b = store.install_from_dir(&second.dir, None).unwrap();
        assert_ne!(a.digest, b.digest);
        assert_eq!(store.list().len(), 2);
        let removed = store.prune(Some(1)).unwrap();
        assert_eq!(removed, 1);
        let left = store.list();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].digest, b.digest);
        assert!(left[0].current);
    }

    #[test]
    fn install_rejects_traversal_and_symlinks() {
        let home = scratch("home");
        let store = Store::at(&home.dir);
        let fixture = pack("bad", "x86_64-unknown-linux-musl", b"runtime");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("lib", fixture.dir.join("alias")).unwrap();
            let err = store.install_from_dir(&fixture.dir, None).unwrap_err();
            assert!(err.to_string().contains("symlink"), "{err}");
        }
        assert!(!store.root().join("..").join("escape").exists());
    }

    #[test]
    fn remove_rejects_path_traversal() {
        let home = scratch("home");
        let store = Store::at(&home.dir);
        for triple in ["../x", "a/b", "", "."] {
            let err = store.remove(triple).unwrap_err();
            assert_eq!(err.code(), diagnostics::Code::E124);
        }
    }
}
