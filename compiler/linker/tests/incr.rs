use linker::{LinkInput, native, plan};
use std::path::{Path, PathBuf};

const MATH: &str = "fn Main(): Int {\n  let a = 40;\n  let b = 2;\n  let c = a + b;\n  return c * 3 - 84;\n}\n";

const MATH_EDIT: &str = "fn Main(): Int {\n  let a = 40;\n  let b = 2;\n  let c = a + b;\n  return c * 3 - 83;\n}\n";

fn fixture_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-incr-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn build_objects(path: &Path, release: bool) -> Vec<Vec<u8>> {
    let mut cfg = cli::CompileConfig::default();
    cfg.jobs = 1;
    cfg.memory_cap = u64::MAX;
    cli::build_files_staged_cfg(path.to_str().unwrap(), "Main", release, 1, None, &cfg)
        .unwrap_or_else(|e| panic!("build {}: {e:?}", path.display()))
        .objects
}

fn static_inputs(objects: &[Vec<u8>]) -> Vec<LinkInput> {
    let mut inputs: Vec<LinkInput> =
        objects.iter().map(|b| LinkInput::ObjectBytes(b.clone())).collect();
    inputs.push(LinkInput::ArchiveBytes(runtime::archive::BYTES.to_vec()));
    for lib in ["pthread", "dl", "m", "c"] {
        inputs.push(LinkInput::Lib(lib.to_string()));
    }
    inputs
}

fn static_opts() -> native::StaticOpts {
    native::StaticOpts {
        entry: "_start".to_string(),
        icf: true,
        target: "x86_64-unknown-linux-gnu".to_string(),
        strip: false,
    }
}

fn link_full(objects: &[Vec<u8>], out: &Path) {
    native::native_link_static(&static_inputs(objects), out, &static_opts()).unwrap();
}

fn link_cached(objects: &[Vec<u8>], out: &Path, ctx: &plan::CacheCtx) {
    let mut inputs: Vec<LinkInput> =
        objects.iter().map(|b| LinkInput::ObjectBytes(b.clone())).collect();
    inputs.push(LinkInput::ArchiveRefKnown {
        bytes: runtime::archive::BYTES,
        sha256: runtime::archive::SHA256,
    });
    for lib in ["pthread", "dl", "m", "c"] {
        inputs.push(LinkInput::Lib(lib.to_string()));
    }
    native::native_link_static_cached(&inputs, out, &static_opts(), ctx).unwrap();
}

fn ctx_for(dir: &Path, allow_write: bool) -> plan::CacheCtx {
    plan::CacheCtx {
        dir: dir.join("link-cache").join("release"),
        toolchain: "incr-test-toolchain".to_string(),
        allow_write,
    }
}

#[test]
fn incremental_link_is_byte_identical_to_full_link() {
    let dir = fixture_dir("identical");
    let path = dir.join("main.rnx");
    std::fs::write(&path, MATH).unwrap();
    let objects = build_objects(&path, true);

    let full = dir.join("full");
    link_full(&objects, &full);
    let full_bytes = std::fs::read(&full).unwrap();

    let ctx = ctx_for(&dir, true);
    let warm1 = dir.join("warm1");
    link_cached(&objects, &warm1, &ctx);
    assert_eq!(
        std::fs::read(&warm1).unwrap(),
        full_bytes,
        "first cached link (plan miss + store) differs from full link"
    );

    let warm2 = dir.join("warm2");
    link_cached(&objects, &warm2, &ctx);
    assert_eq!(
        std::fs::read(&warm2).unwrap(),
        full_bytes,
        "second cached link (plan hit) differs from full link"
    );

    assert!(
        dir.join("link-cache").join("release").join(plan::MANIFEST_NAME).is_file(),
        "plan manifest was not written"
    );
}

#[test]
fn incremental_link_tracks_edited_units() {
    let dir = fixture_dir("edited");
    let path = dir.join("main.rnx");
    std::fs::write(&path, MATH).unwrap();
    let before = build_objects(&path, true);
    let ctx = ctx_for(&dir, true);
    let first = dir.join("first");
    link_cached(&before, &first, &ctx);

    std::fs::write(&path, MATH_EDIT).unwrap();
    let after = build_objects(&path, true);
    let edited_full = dir.join("edited_full");
    link_full(&after, &edited_full);
    let edited_bytes = std::fs::read(&edited_full).unwrap();

    let edited_cached = dir.join("edited_cached");
    link_cached(&after, &edited_cached, &ctx);
    assert_eq!(
        std::fs::read(&edited_cached).unwrap(),
        edited_bytes,
        "cached link of edited units differs from full link"
    );
    assert_ne!(
        edited_bytes,
        std::fs::read(&first).unwrap(),
        "edited program should link to different bytes than the original"
    );

    let status = std::process::Command::new(&edited_cached).output().unwrap();
    assert_eq!(status.status.code(), Some(43), "edited binary exit code changed");
}

#[test]
fn corrupt_plan_fails_open_to_identical_bytes() {
    let dir = fixture_dir("corrupt");
    let path = dir.join("main.rnx");
    std::fs::write(&path, MATH).unwrap();
    let objects = build_objects(&path, true);
    let ctx = ctx_for(&dir, true);
    let seeded = dir.join("seeded");
    link_cached(&objects, &seeded, &ctx);

    std::fs::write(ctx.dir.join(plan::MANIFEST_NAME), b"{\"magic\":\"NOPE\"}").unwrap();
    let recovered = dir.join("recovered");
    link_cached(&objects, &recovered, &ctx);
    assert_eq!(
        std::fs::read(&recovered).unwrap(),
        std::fs::read(&seeded).unwrap(),
        "corrupt plan did not fail open to identical bytes"
    );

    std::fs::remove_dir_all(&ctx.dir).unwrap();
    let cold_ctx = ctx_for(&dir, false);
    let cold = dir.join("cold");
    link_cached(&objects, &cold, &cold_ctx);
    assert_eq!(
        std::fs::read(&cold).unwrap(),
        std::fs::read(&seeded).unwrap(),
        "cold (no-write) cached link differs from seeded link"
    );
    assert!(
        !cold_ctx.dir.join(plan::MANIFEST_NAME).is_file(),
        "cold path with allow_write=false must not write a plan"
    );
}

#[test]
fn toolchain_bump_invalidates_plan() {
    let dir = fixture_dir("toolchain");
    let path = dir.join("main.rnx");
    std::fs::write(&path, MATH).unwrap();
    let objects = build_objects(&path, true);
    let ctx = ctx_for(&dir, true);
    let first = dir.join("first");
    link_cached(&objects, &first, &ctx);

    let mut bumped = ctx_for(&dir, true);
    bumped.toolchain = "incr-test-toolchain-bumped".to_string();
    let second = dir.join("second");
    link_cached(&objects, &second, &bumped);
    assert_eq!(
        std::fs::read(&second).unwrap(),
        std::fs::read(&first).unwrap(),
        "toolchain-bumped link differs in bytes"
    );
}
