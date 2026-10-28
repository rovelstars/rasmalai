#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use llvm::codegen::{baseline_cpu_for, emit_object, host_triple_str, release_ir, version_target_for, OptLevel};

fn lir_of(src: &str, tag: &str) -> lir::instr::Module {
    let dir = crate::scratch::dir(&format!("fmv-{tag}"));
    std::fs::write(dir.join("main.rnx"), src).unwrap();
    let g = frontend::modules::ModuleGraph::build(&dir.join("main.rnx")).unwrap_or_else(|e| panic!("{e}"));
    let mut m = g.resolve().unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    let mut out = lir::lower::lower(&m).unwrap_or_else(|e| panic!("{e}"));
    lir::opt::optimize_lir(&mut out, 1, "Main");
    out
}

#[test]
fn baseline_cpu_is_portable_per_arch() {
    assert_eq!(baseline_cpu_for("x86_64-unknown-linux-gnu"), ("x86-64".to_string(), String::new()));
    assert_eq!(baseline_cpu_for("x86_64-pc-windows-msvc"), ("x86-64".to_string(), String::new()));
    assert_eq!(baseline_cpu_for("aarch64-apple-darwin"), ("generic".to_string(), String::new()));
    assert_eq!(baseline_cpu_for("aarch64-unknown-linux-gnu"), ("generic".to_string(), String::new()));
}

#[test]
fn version_target_covers_x86_64_only() {
    let x86 = version_target_for(Some("x86_64-unknown-linux-gnu"), true).expect("x86-64 gets versions");
    assert_eq!(x86, "x86_64-unknown-linux-gnu");
    assert_eq!(version_target_for(Some("aarch64-unknown-linux-gnu"), true), None);
    assert_eq!(version_target_for(Some("riscv64gc-unknown-linux-gnu"), true), None);
    assert_eq!(version_target_for(Some("x86_64-unknown-linux-gnu"), false), None);
    assert_eq!(version_target_for(None, false), None);
    if std::env::consts::ARCH == "x86_64" {
        assert!(version_target_for(None, true).is_some(), "host x86-64 versions in release");
    } else {
        assert_eq!(version_target_for(None, true), None);
    }
    assert!(!host_triple_str().is_empty(), "host triple known");
}

const LOOP_SRC: &str = "fn sum_to(n: Int): Int {\n    let s = 0;\n    let i = 0;\n    while i < n {\n        s = s + i;\n        i = i + 1;\n    }\n    return s;\n}\nfn Main(): Int {\n    return sum_to(100);\n}\n";

fn has_version_syms(ir: &str, name: &str) -> bool {
    [".v0", ".v2", ".v3", ".v4"].iter().all(|s| ir.contains(&format!("{name}{s}")))
}

#[test]
fn release_ir_versions_loop_functions_on_x86_64() {
    let lir = lir_of(LOOP_SRC, "ir");
    let ir = release_ir(&lir, "fmv_ir", Some("x86_64-unknown-linux-gnu")).unwrap_or_else(|e| panic!("{e}"));
    assert!(has_version_syms(&ir, "Main"), "missing tier bodies:\n{ir}");
    assert!(ir.contains("rnx_cpu_tier"), "stub must probe the cached tier:\n{ir}");
    assert!(ir.contains("\"target-cpu\"=\"x86-64-v3\""), "v3 tier pins its CPU:\n{ir}");
    assert!(ir.contains("\"target-cpu\"=\"x86-64-v2\""), "v2 tier pins its CPU:\n{ir}");
    assert!(ir.contains("\"target-cpu\"=\"x86-64-v4\""), "v4 tier pins its CPU:\n{ir}");
}

#[test]
fn release_ir_stays_single_version_off_x86_64() {
    let lir = lir_of(LOOP_SRC, "irplain");
    let ir = release_ir(&lir, "fmv_plain", Some("aarch64-unknown-linux-gnu")).unwrap_or_else(|e| panic!("{e}"));
    assert!(!ir.contains(".v3"), "non-x86 target must stay single-version:\n{ir}");
    assert!(!ir.contains("rnx_cpu_tier"), "no probe without versions:\n{ir}");
}

#[test]
fn release_object_carries_versioned_symbols_on_x86_64() {
    let lir = lir_of(LOOP_SRC, "obj");
    let bytes = emit_object(&lir, "fmv_obj", "Main", OptLevel::Release, None).unwrap_or_else(|e| panic!("{e:?}"));
    assert!(!bytes.is_empty(), "empty object");
    let has = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
    if std::env::consts::ARCH == "x86_64" {
        assert!(has(b"Main.v3"), "release object has no v3 body ({} bytes)", bytes.len());
        assert!(has(b"rnx_cpu_tier"), "release object has no tier probe ({} bytes)", bytes.len());
        assert!(!has(b".resolver"), "no ifunc resolvers: dispatch is a plain stub");
    } else {
        assert!(!has(b"Main.v3"), "non-x86 host must stay baseline-only");
    }
}

#[test]
fn dev_object_has_no_versions() {
    let lir = lir_of(LOOP_SRC, "objdev");
    let bytes = emit_object(&lir, "fmv_objdev", "Main", OptLevel::Dev, None).unwrap_or_else(|e| panic!("{e:?}"));
    let has = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
    assert!(!has(b"Main.v3"), "dev object must stay single-version baseline");
}


const FLAT_SRC: &str = "fn Main(): Int {\n    return 42;\n}\n";

#[test]
fn release_ir_leaves_loopless_functions_single_version() {
    let lir = lir_of(FLAT_SRC, "flat");
    let ir = release_ir(&lir, "fmv_flat", Some("x86_64-unknown-linux-gnu")).unwrap_or_else(|e| panic!("{e}"));
    assert!(!ir.contains(".v3"), "loopless code must stay single-version:\n{ir}");
    assert!(!ir.contains("rnx_cpu_tier"), "no probe without versions:\n{ir}");
}
