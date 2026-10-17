// Proves the shared scratch guard in `testkit/scratch.rs` removes its tree on
// both the normal and the panicking path, and that names stay unique per run.
#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::path::PathBuf;

fn scratch_dirs(tag: &str) -> Vec<PathBuf> {
    let prefix = format!("rnx-test-{tag}-");
    std::fs::read_dir(std::env::temp_dir())
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(&prefix)))
        .collect()
}

#[test]
fn guard_removes_the_tree_when_it_drops() {
    let path = {
        let dir = scratch::dir("drop");
        std::fs::create_dir_all(dir.join("nested/deeper")).unwrap();
        std::fs::write(dir.join("nested/deeper/f.rnx"), b"fn Main(): Int { return 1 }").unwrap();
        assert!(dir.is_dir());
        dir.path().to_path_buf()
    };
    assert!(!path.exists(), "{} survived the guard drop", path.display());
}

#[test]
fn guard_removes_the_tree_when_the_test_panics() {
    // The guard is a plain RAII value, so the panic payload unwinds past it and
    // its Drop still runs. `catch_unwind` stands in for libtest, which lets the
    // assertion below see the directory is gone.
    let mut created: Option<PathBuf> = None;
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let dir = scratch::dir("panic");
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        std::fs::write(dir.join("nested/marker"), b"x").unwrap();
        created = Some(dir.path().to_path_buf());
        assert!(dir.is_dir());
        panic!("deliberate failure inside a scratch-using test");
    }));
    assert!(outcome.is_err(), "the closure was supposed to panic");
    let path = created.expect("the guard was never created");
    assert!(!path.exists(), "{} survived the unwind", path.display());
}

#[test]
fn names_are_unique_per_process_and_carry_the_pid() {
    let a = scratch::dir("unique");
    let b = scratch::dir("unique");
    assert_ne!(a.path(), b.path());
    let pid = std::process::id().to_string();
    for p in [a.path(), b.path()] {
        let name = p.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("rnx-test-unique-"), "{name}");
        assert!(name.contains(&pid), "{name} lacks pid {pid}");
    }
    drop(a);
    drop(b);
}

#[test]
fn a_dropped_name_is_reclaimed_and_leaves_nothing_behind() {
    let path = scratch::dir("reused").path().to_path_buf();
    drop(scratch::dir("reused"));
    let again = scratch::dir("reused");
    assert!(again.is_dir());
    assert!(scratch_dirs("reused").len() <= 1);
    drop(again);
    assert!(!path.exists());
    assert!(scratch_dirs("reused").is_empty(), "guard left a tree behind");
}