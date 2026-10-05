use diagnostics::Diagnostic;

pub const DEV_PASSES: &[&str] = &["verify"];

pub const RELEASE_PASSES: &[&str] = &[
    "inline",
    "escape",
    "sroa",
    "licm",
    "bce",
    "arc",
    "tco",
    "fixpoint",
    "sweep",
    "deadfn",
    "verify",
];

pub fn dev_passes() -> Vec<&'static str> {
    DEV_PASSES.to_vec()
}

pub fn release_passes() -> Vec<&'static str> {
    RELEASE_PASSES.to_vec()
}

pub fn dev_is_strict_subset_of_release() -> bool {
    if DEV_PASSES.len() >= RELEASE_PASSES.len() {
        return false;
    }
    DEV_PASSES
        .iter()
        .all(|pass| RELEASE_PASSES.contains(pass))
}

pub fn optimize_for_dev(module: &mut lir::instr::Module, entry: &str) -> Vec<Diagnostic> {
    lir::opt::optimize_lir(module, 0, entry);
    lir::verify::verify(module)
}

pub fn optimize_for_release(module: &mut lir::instr::Module, entry: &str) -> Vec<Diagnostic> {
    lir::opt::optimize_lir(module, 1, entry);
    lir::verify::verify(module)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn dev_pipeline_is_strict_subset_of_release() {
        assert!(dev_is_strict_subset_of_release());
        let dev: BTreeSet<&str> = dev_passes().into_iter().collect();
        let release: BTreeSet<&str> = release_passes().into_iter().collect();
        assert!(dev.is_subset(&release));
        assert!(dev.len() < release.len());
        assert!(release.contains("verify"));
    }

    #[test]
    fn release_keeps_full_pipeline() {
        let passes = release_passes();
        for expensive in [
            "inline",
            "escape",
            "sroa",
            "licm",
            "bce",
            "arc",
            "tco",
            "fixpoint",
            "sweep",
            "deadfn",
        ] {
            assert!(passes.contains(&expensive), "release drops {expensive}");
        }
        assert!(!dev_passes().iter().any(|p| *p != "verify"));
    }

    fn lowered(src: &str) -> lir::instr::Module {
        let mut module = frontend::modules::ModuleGraph::from_source(src).unwrap();
        assert!(frontend::desugar::desugar(&mut module).is_empty());
        lir::lower::lower(&module).unwrap()
    }

    #[test]
    fn both_pipelines_verify_clean() {
        let mut dev = lowered("fn Fib(n: Int): Int { if n < 2 { return n; } return Fib(n - 1) + Fib(n - 2); } fn Main(): Int { return Fib(10); }");
        let mut release = lowered("fn Fib(n: Int): Int { if n < 2 { return n; } return Fib(n - 1) + Fib(n - 2); } fn Main(): Int { return Fib(10); }");
        assert!(optimize_for_dev(&mut dev, "Main").is_empty());
        assert!(optimize_for_release(&mut release, "Main").is_empty());
        for module in [&dev, &release] {
            assert!(module.functions.iter().any(|f| f.name == "Main"));
            assert!(module.functions.iter().any(|f| f.name == "Fib"));
        }
    }
}
