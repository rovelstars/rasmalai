//! Scratch directories that do not outlive the test run.
//!
//! [`dir`] creates a unique directory under the system temp directory and
//! hands back a guard that removes the whole tree when it drops. A test that
//! panics unwinds through the guard, so a failing run leaves nothing behind
//! either.
//!
//! Uniqueness comes from [`tempfile`]: every call mints a fresh random
//! suffix, so two tests in the same process, two processes on the same host,
//! and two hosts sharing a temp filesystem never share one directory. The
//! directory name still carries the `rnx-test-{tag}-{pid}-` prefix for
//! triage (`ls $TMPDIR | grep rnx-test-…`).
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
//!
//! Rule for test filesystem use: never write to a hardcoded, shared, or
//! repository-relative path (`/tmp/…`, `target/…`, fixture trees). Always build
//! the tree under `dir()` and run the subject with that directory as its
//! working directory or via absolute paths into it.

use std::path::Path;

use tempfile::TempDir;

/// Creates a scratch directory tagged `tag` and removes it when the guard drops.
pub fn dir(tag: &str) -> Scratch {
    let prefix = format!("rnx-test-{tag}-{}-", std::process::id());
    let temp = tempfile::Builder::new()
        .prefix(&prefix)
        .tempdir()
        .unwrap_or_else(|e| panic!("scratch {tag}: create tempdir: {e}"));
    Scratch { temp }
}

/// A scratch directory that removes itself on drop.
#[derive(Debug)]
pub struct Scratch {
    temp: TempDir,
}

impl Scratch {
    // This file is compiled into every test target that uses it, and most
    // targets reach the path only through Deref, so `path` would read as dead
    // code in all of them.
    #[allow(dead_code)]
    pub fn path(&self) -> &Path {
        self.temp.path()
    }
}

impl AsRef<Path> for Scratch {
    fn as_ref(&self) -> &Path {
        self.temp.path()
    }
}

// Deref lets a helper keep its original body: a `Scratch` binding stands in for
// the `PathBuf` it replaced, so `dir.join(..)` and `&dir` at a `&Path`
// parameter keep working while the guard still drops with the test.
impl std::ops::Deref for Scratch {
    type Target = Path;

    fn deref(&self) -> &Path {
        self.temp.path()
    }
}
