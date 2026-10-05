use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, src).unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap_or_else(|e| panic!("{e}"));
    let mut m = g.resolve().unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    let mut out = lir::lower::lower(&m).unwrap_or_else(|e| panic!("{e}"));
    lir::opt::optimize_lir(&mut out, 1, "Main");
    let v = lir::verify::verify(&out);
    assert!(v.is_empty(), "{v:?}");
    (out, dir)
}

fn lower_only(src: &str, tag: &str) -> lir::instr::Module {
    let dir = std::env::temp_dir().join(format!("rnx-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, src).unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap_or_else(|e| panic!("{e}"));
    let mut m = g.resolve().unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    let out = lir::lower::lower(&m).unwrap_or_else(|e| panic!("{e}"));
    let _ = std::fs::remove_dir_all(&dir);
    out
}

fn check_all_backends(src: &str, want: i64, want_out: &[String], tag: &str) {
    let (module, dir) = resolve_src(src, tag);
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(module));
    let mut machine = runtime::machine::Machine::new(leaked);
    let r = machine.call("Main", vec![]).unwrap_or_else(|e| panic!("interpreter {tag}: {e:?}"));
    match r {
        runtime::value::Value::Int(v) if v == want => {}
        other => panic!("interpreter {tag}: {other:?}"),
    }
    let got = machine.output.clone();
    let want_out: Vec<String> = want_out.to_vec();
    assert_eq!(got, want_out, "interpreter {tag} stdout");

    let mut jit = cranelift::jit::Jit::compile(leaked).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), want, "cranelift {tag}");
    assert_eq!(llvm::codegen::execute(leaked, "Main").unwrap(), want, "llvm {tag}");

    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(dir.join("main.rnx"))
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = String::from_utf8_lossy(&build.stdout).lines().rev().find_map(|l| l.strip_prefix("artifact: ")).map(|s| s.trim().trim_end_matches(" (fresh)").to_string()).expect("build prints artifact path");
    let run = std::process::Command::new(&bin).output().unwrap();
    assert_eq!(run.status.code().unwrap(), want as i32, "aot {tag} exit");
    let suffix = if want_out.is_empty() { "" } else { "\n" };
    assert_eq!(
        String::from_utf8(run.stdout).unwrap(),
        want_out.join("\n") + suffix,
        "aot {tag} stdout"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

const ARITH_SRC: &str = "import { Vec4f, Vec4i } from \"@std/simd\";\n\nfn Main(): Int {\n    let v = new Vec4f(1.0, 2.0, 3.0, 4.0) + new Vec4f(10.0, 20.0, 30.0, 40.0);\n    let a = Int(v.x());\n    let b = Int(v.y());\n    let c = Int(v.get(2));\n    let d = Int(v.w());\n    let w = new Vec4i(1, 2, 3, 4) * new Vec4i(2, 2, 2, 2);\n    let e = w.get(0) + w.get(3);\n    print(\"lanes:\", a, b, c, d, e);\n    if a == 11 && b == 22 && c == 33 && d == 44 && e == 10 {\n        return 42;\n    }\n    return 1;\n}\n";

#[test]
fn test_vec4f_arithmetic_and_lanes() {
    check_all_backends(ARITH_SRC, 42, &["lanes: 11 22 33 44 10".to_string()], "vecarith");
}

const DOT_SRC: &str = "import { Vec4f } from \"@std/simd\";\n\nfn Main(): Int {\n    let d = new Vec4f(1.0, 2.0, 3.0, 4.0).dot(new Vec4f(2.0, 3.0, 4.0, 5.0));\n    let n = Int(d);\n    print(\"dot:\", n);\n    if n == 40 {\n        return 42;\n    }\n    return 1;\n}\n";

#[test]
fn test_vec4f_dot_product() {
    check_all_backends(DOT_SRC, 42, &["dot: 40".to_string()], "vecdot");
}

const SQRT_SRC: &str = "import { Vec4f } from \"@std/simd\";\n\nfn Main(): Int {\n    let r = new Vec4f(4.0, 9.0, 16.0, 25.0).sqrt();\n    let a = Int(r.x()) + Int(r.y()) + Int(r.z()) + Int(r.w());\n    let lo = new Vec4f(1.0, 5.0, 3.0, 4.0).min(new Vec4f(4.0, 2.0, 6.0, 0.0));\n    let b = Int(lo.x()) + Int(lo.y()) + Int(lo.z()) + Int(lo.w());\n    let hi = new Vec4f(1.0, 5.0, 3.0, 4.0).max(new Vec4f(4.0, 2.0, 6.0, 0.0));\n    let c = Int(hi.x()) + Int(hi.y()) + Int(hi.z()) + Int(hi.w());\n    print(\"vec:\", a, b, c);\n    if a == 14 && b == 6 && c == 19 {\n        return 42;\n    }\n    return 1;\n}\n";

#[test]
fn test_vec4f_sqrt_and_min_max() {
    check_all_backends(SQRT_SRC, 42, &["vec: 14 6 19".to_string()], "vecsqrt");
}

const POOL_SRC: &str = "import { AtomicInt } from \"@std/sync\";\nimport { Vec4f } from \"@std/simd\";\n\nfn work(c: Int) {\n    let j = c * 4;\n    let a0 = Float(AtomicInt.byId(2000 + j).get());\n    let a1 = Float(AtomicInt.byId(2000 + j + 1).get());\n    let a2 = Float(AtomicInt.byId(2000 + j + 2).get());\n    let a3 = Float(AtomicInt.byId(2000 + j + 3).get());\n    let v = new Vec4f(a0, a1, a2, a3) * Vec4f.splat(2.0) + Vec4f.splat(1.0);\n    AtomicInt.byId(5000 + j).set(Int(v.get(0)));\n    AtomicInt.byId(5000 + j + 1).set(Int(v.get(1)));\n    AtomicInt.byId(5000 + j + 2).set(Int(v.get(2)));\n    AtomicInt.byId(5000 + j + 3).set(Int(v.get(3)));\n}\n\nfn Main(): Int {\n    let i = 0;\n    while i < 1024 {\n        AtomicInt.byId(2000 + i).set(i);\n        i = i + 1;\n    }\n    let pool = ThreadPool.byId(705, 4);\n    pool.parallelFor(0, 256, 16, work);\n    pool.join();\n    pool.shutdown();\n    let s = 0;\n    let j = 0;\n    while j < 1024 {\n        s = s + AtomicInt.byId(5000 + j).get();\n        j = j + 1;\n    }\n    print(\"simd pool:\", s);\n    if s == 1048576 {\n        return 42;\n    }\n    return 1;\n}\n";

#[test]
fn test_simd_parallel_workload_matrix() {
    check_all_backends(POOL_SRC, 42, &["simd pool: 1048576".to_string()], "vecpool");
}

const ARC_SRC: &str = "import { Vec4f, Vec4i } from \"@std/simd\";\n\nfn Main(): Int {\n    let v = new Vec4f(1.0, 2.0, 3.0, 4.0) + Vec4f.splat(1.0);\n    let w = new Vec4i(1, 2, 3, 4) * Vec4i.splat(3);\n    let d = v.dot(v);\n    let r = v.sqrt().min(v.max(v));\n    let s = Int(v.get(0)) + w.get(0) + Int(d) + Int(r.x());\n    return s;\n}\n";

#[test]
fn test_zero_arc_overhead() {
    let m = lower_only(ARC_SRC, "vecarc");
    let mut retains = 0;
    let mut releases = 0;
    let mut vec_ops = 0;
    for f in &m.functions {
        if f.name != "Main" {
            continue;
        }
        if f.name.starts_with("__ext_") {
            continue;
        }
        for b in &f.blocks {
            for ins in &b.instrs {
                match ins {
                    lir::instr::Instr::Retain { .. } => retains += 1,
                    lir::instr::Instr::Release { .. } => releases += 1,
                    lir::instr::Instr::VecNew { .. }
                    | lir::instr::Instr::VecSplat { .. }
                    | lir::instr::Instr::VecExtract { .. }
                    | lir::instr::Instr::VecArith { .. }
                    | lir::instr::Instr::VecUnary { .. }
                    | lir::instr::Instr::VecDot { .. } => vec_ops += 1,
                    _ => {}
                }
            }
        }
    }
    assert!(vec_ops > 0, "expected vector instructions in lowered LIR");
    assert_eq!((retains, releases), (0, 0), "SIMD ops must not emit ARC traffic");
}

const NEG_SRC: &str = "import { Vec4i } from \"@std/simd\";\n\nfn Main(): Int {\n    let w = new Vec4i(0, 0, 0, 0) - new Vec4i(1, 1, 1, 1);\n    let got = w.get(0);\n    let want = 0 - 1;\n    print(\"neg:\", got);\n    if got == want {\n        return 42;\n    }\n    return 1;\n}\n";

#[test]
fn test_vec4i_negative_lane_sign_extends() {
    check_all_backends(NEG_SRC, 42, &["neg: -1".to_string()], "vecneg");
}

const NAN_SRC: &str = "import { Vec4f } from \"@std/simd\";\n\nfn Main(): Int {\n    let n = 0.0 / 0.0;\n    let lo = new Vec4f(n, 7.0, 1.0, 2.0).min(new Vec4f(5.0, n, 0.0, 1.0));\n    let hi = new Vec4f(n, 7.0, 1.0, 2.0).max(new Vec4f(5.0, n, 0.0, 1.0));\n    print(\"nan:\", lo.x(), lo.y(), hi.x(), hi.y());\n    if lo.x() == 5.0 && lo.y() == 7.0 && hi.x() == 5.0 && hi.y() == 7.0 {\n        return 42;\n    }\n    return 1;\n}\n";

#[test]
fn test_vec4f_min_max_nan_returns_other_lane() {
    check_all_backends(NAN_SRC, 42, &["nan: 5.0 7.0 5.0 7.0".to_string()], "vecnan");
}

const OOB_HIGH_SRC: &str = "import { Vec4f } from \"@std/simd\";\n\nfn Main(): Int {\n    let v = new Vec4f(1.0, 2.0, 3.0, 4.0);\n    let i = 4;\n    print(v.get(i));\n    return 42;\n}\n";

const OOB_NEG_SRC: &str = "import { Vec4f } from \"@std/simd\";\n\nfn Main(): Int {\n    let v = new Vec4f(1.0, 2.0, 3.0, 4.0);\n    let i = 0 - 1;\n    print(v.get(i));\n    return 42;\n}\n";

fn check_oob_traps(src: &str, tag: &str) {
    let out = cli::run_source(src, "Main", vec![]);
    match out.outcome {
        cli::RunOutcome::Fatal { message, .. } => assert!(message.contains("out of range"), "{message}"),
        other => panic!("interpreter {tag}: {other:?}"),
    }
    let dir = std::env::temp_dir().join(format!("rnx-vecoob-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src_path = dir.join("oob.rnx");
    std::fs::write(&src_path, src).unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    for backend in ["cranelift", "llvm"] {
        let run = std::process::Command::new(rnx)
            .arg("run")
            .arg("--backend")
            .arg(backend)
            .arg(&src_path)
            .output()
            .unwrap();
        assert!(!run.status.success(), "{backend} {tag} read past the end without trapping");
        let stderr = String::from_utf8_lossy(&run.stderr).into_owned();
        assert!(stderr.contains("vector lane out of range"), "{backend} {tag}: {stderr}");
    }
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(&src_path)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = String::from_utf8_lossy(&build.stdout).lines().rev().find_map(|l| l.strip_prefix("artifact: ")).map(|s| s.trim().trim_end_matches(" (fresh)").to_string()).expect("build prints artifact path");
    let run = std::process::Command::new(&bin).output().unwrap();
    assert!(!run.status.success(), "aot {tag} read past the end without trapping");
    let stderr = String::from_utf8_lossy(&run.stderr).into_owned();
    assert!(stderr.contains("vector lane out of range"), "aot {tag}: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_vec4f_oob_lane_high_traps_on_all_backends() {
    check_oob_traps(OOB_HIGH_SRC, "vecoob-high");
}

#[test]
fn test_vec4f_oob_lane_negative_traps_on_all_backends() {
    check_oob_traps(OOB_NEG_SRC, "vecoob-neg");
}
