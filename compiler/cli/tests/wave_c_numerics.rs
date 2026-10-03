use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-wavec-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, src).unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap_or_else(|e| panic!("{e}"));
    let mut m = g.resolve().unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    let c = frontend::semantic::check(&m);
    assert!(c.iter().all(|x| x.code.is_warning()), "{c:?}");
    let mut out = lir::lower::lower(&m).unwrap_or_else(|e| panic!("{e}"));
    lir::opt::optimize_lir(&mut out, 1, "Main");
    let v = lir::verify::verify(&out);
    assert!(v.is_empty(), "{v:?}");
    (out, dir)
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

    let bin_path = dir.join("wavec_bin");
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(dir.join("main.rnx"))
        .arg("-o")
        .arg(&bin_path)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new(&bin_path).output().unwrap();
    assert_eq!(run.status.code().unwrap(), want as i32, "aot {tag} exit");
    let suffix = if want_out.is_empty() { "" } else { "\n" };
    assert_eq!(
        String::from_utf8(run.stdout).unwrap(),
        want_out.join("\n") + suffix,
        "aot {tag} stdout"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

const INF_BITS: &str = "9218868437227405312";
const TEN_BITS: &str = "4621819117588971520";
const ONE_PT_FIVE_BITS: &str = "4609434218613702656";

#[test]
fn nan_is_semantic_on_all_backends() {
    check_all_backends(
        "import { Math } from \"@std/math\";\n\
        fn Main(): Int {\n\
        let z: Float = 0.0;\n\
        let d: Float = z / z;\n\
        print(Float.isNaN(d));\n\
        print(d != d);\n\
        print(d == d);\n\
        let n: Float = Float.nan();\n\
        print(Float.isNaN(n));\n\
        print(n != n);\n\
        let s: Float = Math.sqrt(0.0 - 1.0);\n\
        print(Float.isNaN(s));\n\
        print(s != s);\n\
        return 0;\n\
        }\n",
        0,
        &[
            "true".to_string(),
            "true".to_string(),
            "false".to_string(),
            "true".to_string(),
            "true".to_string(),
            "true".to_string(),
            "true".to_string(),
        ],
        "nan-semantic",
    );
}

#[test]
fn float_bit_roundtrip_on_all_backends() {
    check_all_backends(
        "import { ByteBuffer } from \"@std/bytes\";\n\
        fn Main(): Int {\n\
        let buf = ByteBuffer.allocate(16);\n\
        buf.writeFloat64BE(0, Float.nan());\n\
        print(Float.isNaN(buf.readFloat64BE(0)));\n\
        buf.writeFloat64BE(0, 1.0 / 0.0);\n\
        print(buf.readInt64BE(0));\n\
        buf.writeFloat64LE(8, Float.fromBits(1));\n\
        print(buf.readInt64LE(8));\n\
        let a = [1.5, Float.nan()];\n\
        print(a[0].toBits());\n\
        print(Float.isNaN(a[1]));\n\
        print(Float.fromBits(a[0].toBits()).toBits());\n\
        print(Float.fromBits(1).toBits());\n\
        return 0;\n\
        }\n",
        0,
        &[
            "true".to_string(),
            INF_BITS.to_string(),
            "1".to_string(),
            ONE_PT_FIVE_BITS.to_string(),
            "true".to_string(),
            ONE_PT_FIVE_BITS.to_string(),
            "1".to_string(),
        ],
        "bit-roundtrip",
    );
}

#[test]
fn fma_is_fused_without_intermediate_overflow() {
    check_all_backends(
        "fn Main(): Int {\n\
        print(Float.fma(2.0, 3.0, 4.0).toBits());\n\
        let big: Float = 10.0;\n\
        let i: Int = 0;\n\
        while i < 307 {\n\
        big = big * 10.0;\n\
        i = i + 1;\n\
        }\n\
        let r: Float = Float.fma(big, 2.0, 0.0 - big);\n\
        print(r.toBits() == big.toBits());\n\
        return 0;\n\
        }\n",
        0,
        &[TEN_BITS.to_string(), "true".to_string()],
        "fma-fused",
    );
}

#[test]
fn nan_inequality_matches_ieee_on_all_backends() {
    check_all_backends(
        "fn Main(): Int {\n\
        let n: Float = Float.nan();\n\
        let m: Float = 1.5;\n\
        print(n != n);\n\
        print(n == n);\n\
        print(n != m);\n\
        print(Float.isNaN(1.5));\n\
        return 0;\n\
        }\n",
        0,
        &[
            "true".to_string(),
            "false".to_string(),
            "true".to_string(),
            "false".to_string(),
        ],
        "nan-ieee",
    );
}

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-wavec-check-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run_check(src: &str, tag: &str) -> std::process::Output {
    let dir = fresh_dir(tag);
    std::fs::write(
        dir.join("Project.config"),
        "export default {\n    project: {\n        name: \"wavec\",\n        version: \"0.1.0\"\n    }\n}\n",
    )
    .unwrap();
    let src_dir = dir.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(src_dir.join("main.rnx"), src).unwrap();
    std::process::Command::new(rnx())
        .arg("check")
        .env("NO_COLOR", "1")
        .current_dir(&dir)
        .arg("src/main.rnx")
        .output()
        .unwrap()
}

#[test]
fn float_static_api_rejects_bad_calls_at_check() {
    let out = run_check(
        "fn Main(): Int {\n    let n: Float = Float.bogus(1.0);\n    return 0;\n}\n",
        "unknown",
    );
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("unknown static `Float.bogus`"), "got: {stderr}");

    let out = run_check(
        "fn Main(): Int {\n    let f: Float = Float.fma(1.0, 2.0);\n    return 0;\n}\n",
        "arity",
    );
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("`Float.fma` takes 3 args"), "got: {stderr}");

    let out = run_check(
        "fn Main(): Int {\n    let x: Float = 1.5;\n    print(x.toBits(1));\n    return 0;\n}\n",
        "tobits-arity",
    );
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("E108"), "got: {stderr}");
}

#[test]
fn math_log_domain_on_all_backends() {
    check_all_backends(
        "import { Math } from \"@std/math\";\n\
        fn Main(): Int {\n\
        print(Math.log(1.0));\n\
        print(Math.log(0.0));\n\
        print(Float.isNaN(Math.log(0.0 - 5.0)));\n\
        print(Math.log(Math.E()) > 0.999 && Math.log(Math.E()) < 1.001);\n\
        print(Math.log(10.0) > 2.302 && Math.log(10.0) < 2.303);\n\
        return 0;\n\
        }\n",
        0,
        &[
            "0.0".to_string(),
            "-inf".to_string(),
            "true".to_string(),
            "true".to_string(),
            "true".to_string(),
        ],
        "math-log",
    );
}
