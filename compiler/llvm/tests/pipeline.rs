use llvm::codegen::{OptLevel, stdlib_prebuilt};

fn lowered(src: &str) -> lir::instr::Module {
    let mut module = frontend::modules::ModuleGraph::from_source(src).unwrap();
    assert!(frontend::desugar::desugar(&mut module).is_empty());
    lir::lower::lower(&module).unwrap()
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rnx-llvm-pipeline-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn dev_pipeline_is_strict_subset_of_release() {
    assert!(runtime::lir_pipeline::dev_is_strict_subset_of_release());
    let dev = runtime::lir_pipeline::dev_passes();
    let release = runtime::lir_pipeline::release_passes();
    assert!(dev.iter().all(|p| release.contains(p)));
    assert!(dev.len() < release.len());
    assert_eq!(dev, vec!["verify"]);
}

#[test]
fn backend_opt_levels_stay_split() {
    assert_ne!(OptLevel::Dev, OptLevel::Release);
}

#[test]
fn both_pipelines_verify_clean() {
    let src = "fn Fib(n: Int): Int { if n < 2 { return n; } return Fib(n - 1) + Fib(n - 2); } fn Main(): Int { return Fib(10); }";
    let mut dev = lowered(src);
    let mut release = lowered(src);
    assert!(runtime::lir_pipeline::optimize_for_dev(&mut dev, "Main").is_empty());
    assert!(runtime::lir_pipeline::optimize_for_release(&mut release, "Main").is_empty());
    for module in [&dev, &release] {
        assert!(module.functions.iter().any(|f| f.name == "Main"));
    }
}

#[test]
fn missing_prebuilt_stdlib_falls_back() {
    let root = scratch("missing");
    assert_eq!(runtime::stdlib_cache::cached_stdlib_in(&root), None);
    assert!(runtime::stdlib_cache::open_cached_stdlib_in(&root).is_none());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn stdlib_cache_key_is_stable() {
    let first = runtime::stdlib_cache::stdlib_cache_key();
    assert_eq!(first.len(), 64);
    assert_eq!(runtime::stdlib_cache::stdlib_cache_key(), first);
}

#[test]
fn stdlib_entry_source_resolves_all_modules() {
    let mut module =
        frontend::modules::ModuleGraph::from_source(stdlib_prebuilt::stdlib_entry_source()).unwrap();
    assert!(frontend::desugar::desugar(&mut module).is_empty());
    let lowered = lir::lower::lower(&module).unwrap();
    assert!(lowered.functions.len() > 100);
}

#[test]
fn ensure_prebuilt_stdlib_builds_and_caches() {
    if !cfg!(target_os = "linux") {
        return;
    }
    let root = scratch("ensure");
    let first = stdlib_prebuilt::ensure_prebuilt_stdlib_in(&root).unwrap();
    assert!(first.is_file());
    assert_eq!(runtime::stdlib_cache::cached_stdlib_in(&root), Some(first.clone()));
    let second = stdlib_prebuilt::ensure_prebuilt_stdlib_in(&root).unwrap();
    assert_eq!(first, second);
    let _ = std::fs::remove_dir_all(&root);
}
