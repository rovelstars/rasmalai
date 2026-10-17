#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use cranelift::jit::{prebuilt_stdlib_status_in, PrebuiltStatus};

fn lowered(src: &str) -> lir::instr::Module {
    let mut module = frontend::modules::ModuleGraph::from_source(src).unwrap();
    assert!(frontend::desugar::desugar(&mut module).is_empty());
    lir::lower::lower(&module).unwrap()
}

#[test]
fn missing_prebuilt_stdlib_falls_back_to_inline() {
    let lir = lowered("fn Main(): Int { return 42; }");
    let root = crate::scratch::dir("cl");
    assert!(matches!(
        prebuilt_stdlib_status_in(&lir, &root),
        PrebuiltStatus::Fallback(_)
    ));
}

#[test]
fn dev_lir_pipeline_is_strict_subset_of_release() {
    assert!(runtime::lir_pipeline::dev_is_strict_subset_of_release());
}
