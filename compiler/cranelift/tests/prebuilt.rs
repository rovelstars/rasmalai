use cranelift::jit::{prebuilt_stdlib_status_in, PrebuiltStatus};

fn lowered(src: &str) -> lir::instr::Module {
    let mut module = frontend::modules::ModuleGraph::from_source(src).unwrap();
    assert!(frontend::desugar::desugar(&mut module).is_empty());
    lir::lower::lower(&module).unwrap()
}

#[test]
fn missing_prebuilt_stdlib_falls_back_to_inline() {
    let lir = lowered("fn Main(): Int { return 42; }");
    let root = std::env::temp_dir().join(format!(
        "rnx-cl-prebuilt-it-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    assert!(matches!(
        prebuilt_stdlib_status_in(&lir, &root),
        PrebuiltStatus::Fallback(_)
    ));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn dev_lir_pipeline_is_strict_subset_of_release() {
    assert!(runtime::lir_pipeline::dev_is_strict_subset_of_release());
}
