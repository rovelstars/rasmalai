//! Scratch directories that do not outlive the test run.
//!
//! [`dir`] builds a path under the system temp directory whose name carries the
//! process id and a per-process counter, so two concurrent suite runs never
//! share one directory, then hands back a guard that removes the whole tree when
//! it drops. A test that panics unwinds through the guard, so a failing run
//! leaves nothing behind either.
//!
//! A test target pulls this in at its crate root:
//!
//! ```ignore
//! #[cfg(test)]
//! #[path = "../testkit/scratch.rs"]
//! mod scratch;
//! ```
//!
//! Sites then read `crate::scratch::dir("tag")`.
//!
//! The attribute has to sit at the crate root rather than inside a `mod tests`
//! block. Nested there, its `..` segments resolve against that module's own
//! directory, which does not exist, and the include fails with ENOENT.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Creates a scratch directory tagged `tag` and removes it when the guard drops.
pub fn dir(tag: &str) -> Scratch {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!("rnx-test-{tag}-{}-{n}", std::process::id()));
    // The pid and counter already keep concurrent runs apart, but a recycled pid
    // can still name a tree an earlier run left behind.
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path)
        .unwrap_or_else(|e| panic!("scratch {tag}: create {}: {e}", path.display()));
    Scratch { path }
}

/// A scratch directory that removes itself on drop.
#[derive(Debug)]
pub struct Scratch {
    path: PathBuf,
}

impl Scratch {
    // This file is compiled into every test target that uses it, and most
    // targets reach the path only through Deref, so `path` would read as dead
    // code in all of them.
    #[allow(dead_code)]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl AsRef<Path> for Scratch {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

// Deref lets a helper keep its original body: a `Scratch` binding stands in for
// the `PathBuf` it replaced, so `dir.join(..)` and `&dir` at a `&Path`
// parameter keep working while the guard still drops with the test.
impl std::ops::Deref for Scratch {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // One of these trees can hold a 30 MB runtime archive, and a failed
        // removal must not replace the test's own failure with a second one.
        let _ = std::fs::remove_dir_all(&self.path);
    }
}