use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-conmap-{tag}-{}", std::process::id()));
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

    let bin_path = dir.join("conmap_bin");
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
    match run.status.code() {
        Some(code) => assert_eq!(code, want as i32, "aot {tag} exit"),
        None => panic!("aot {tag} killed by signal: {run:?}"),
    }
    let suffix = if want_out.is_empty() { "" } else { "\n" };
    assert_eq!(
        String::from_utf8(run.stdout).unwrap(),
        want_out.join("\n") + suffix,
        "aot {tag} stdout"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

const CHURN_SRC: &str = "import { Map } from \"@std/collections\";\nfn churn(m: Map, iters: Int): Int {\n    let i = 0;\n    while i < iters {\n        m.set(7, i);\n        let v = m.get(7) ?? -1;\n        if v < 0 {\n            return 99;\n        }\n        i = i + 1;\n    }\n    return 0;\n}\nfn Main(): Int {\n    let m = new Map();\n    m.set(7, 1);\n    let h0 = Thread.spawn((): Int => churn(m, 2000));\n    let h1 = Thread.spawn((): Int => churn(m, 2000));\n    let h2 = Thread.spawn((): Int => churn(m, 2000));\n    let h3 = Thread.spawn((): Int => churn(m, 2000));\n    let r0 = h0.join().unwrap();\n    let r1 = h1.join().unwrap();\n    let r2 = h2.join().unwrap();\n    let r3 = h3.join().unwrap();\n    if r0 + r1 + r2 + r3 != 0 {\n        return 3;\n    }\n    if m.len() != 1 {\n        return 1;\n    }\n    let v = m.get(7) ?? -1;\n    if v < 0 {\n        return 2;\n    }\n    print(\"map churn ok\");\n    return 42;\n}\n";

#[test]
fn test_concurrent_map_shared_key_churn() {
    check_all_backends(CHURN_SRC, 42, &["map churn ok".to_string()], "conchurn");
}

const DISTINCT_SRC: &str = "import { Map } from \"@std/collections\";\nfn distinct(m: Map, base: Int, n: Int): Int {\n    let i = 0;\n    while i < n {\n        m.set(base + i, base + i);\n        i = i + 1;\n    }\n    let j = 0;\n    while j < 50 {\n        m.set(\"sk\" + __rnx_int_to_str(j), base);\n        j = j + 1;\n    }\n    return 0;\n}\nfn Main(): Int {\n    let m = new Map();\n    let h0 = Thread.spawn((): Int => distinct(m, 0, 250));\n    let h1 = Thread.spawn((): Int => distinct(m, 1000, 250));\n    let h2 = Thread.spawn((): Int => distinct(m, 2000, 250));\n    let h3 = Thread.spawn((): Int => distinct(m, 3000, 250));\n    let r0 = h0.join().unwrap();\n    let r1 = h1.join().unwrap();\n    let r2 = h2.join().unwrap();\n    let r3 = h3.join().unwrap();\n    if r0 + r1 + r2 + r3 != 0 {\n        return 4;\n    }\n    if m.len() != 1050 {\n        return 1;\n    }\n    let b = 0;\n    while b < 4 {\n        let base = b * 1000;\n        let i = 0;\n        while i < 250 {\n            let v = m.get(base + i) ?? -1;\n            if v != base + i {\n                return 2;\n            }\n            i = i + 1;\n        }\n        b = b + 1;\n    }\n    let j = 0;\n    while j < 50 {\n        let v = m.get(\"sk\" + __rnx_int_to_str(j)) ?? -1;\n        if v < 0 {\n            return 3;\n        }\n        j = j + 1;\n    }\n    print(\"map distinct ok\");\n    return 42;\n}\n";

#[test]
fn test_concurrent_map_distinct_and_shared_keys() {
    check_all_backends(DISTINCT_SRC, 42, &["map distinct ok".to_string()], "condistinct");
}

const DELREAD_SRC: &str = "import { Map } from \"@std/collections\";\nfn deleter(m: Map, base: Int, n: Int): Int {\n    let i = 0;\n    while i < n {\n        m.delete(base + i);\n        i = i + 1;\n    }\n    return 0;\n}\nfn reader(m: Map, rounds: Int): Int {\n    let r = 0;\n    while r < rounds {\n        let k = 0;\n        while k < 400 {\n            let v = m.get(k) ?? -1;\n            if v != -1 {\n                if v != k * 3 + 1 {\n                    return 7;\n                }\n            }\n            k = k + 1;\n        }\n        r = r + 1;\n    }\n    return 0;\n}\nfn Main(): Int {\n    let m = new Map();\n    let k = 0;\n    while k < 400 {\n        m.set(k, k * 3 + 1);\n        k = k + 1;\n    }\n    let h0 = Thread.spawn((): Int => deleter(m, 0, 200));\n    let h1 = Thread.spawn((): Int => deleter(m, 200, 200));\n    let h2 = Thread.spawn((): Int => reader(m, 10));\n    let h3 = Thread.spawn((): Int => reader(m, 10));\n    let r0 = h0.join().unwrap();\n    let r1 = h1.join().unwrap();\n    let r2 = h2.join().unwrap();\n    let r3 = h3.join().unwrap();\n    if r0 + r1 + r2 + r3 != 0 {\n        return 5;\n    }\n    if m.len() != 0 {\n        return 1;\n    }\n    print(\"map delread ok\");\n    return 42;\n}\n";

#[test]
fn test_concurrent_map_delete_read_overlap() {
    check_all_backends(DELREAD_SRC, 42, &["map delread ok".to_string()], "condelread");
}
