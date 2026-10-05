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

fn lower_err(src: &str, tag: &str) -> diagnostics::Diagnostic {
    let dir = std::env::temp_dir().join(format!("rnx-fsmmap-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, src).unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap_or_else(|e| panic!("{e}"));
    let mut m = g.resolve().unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    let err = lir::lower::lower(&m).expect_err("expected lowering error");
    let _ = std::fs::remove_dir_all(&dir);
    err
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

fn mmap_src(dir: &PathBuf) -> String {
    let d = dir.to_string_lossy();
    format!(
        "import fs, {{ Mmap }} from \"@std/fs\";\nimport {{ Result }} from \"@std/prelude\";\n\nunsafe fn filePart(base: String): Int {{\n    let missing: Result<Mmap, String> = fs.mmap(base + \"/gone.bin\", .ReadWrite);\n    if missing.isOk() {{ return 1; }}\n    let got: Result<Mmap, String> = fs.mmap(base + \"/seg.bin\", .ReadWrite);\n    if got.isErr() {{ return 2; }}\n    let m: Mmap = got.unwrap();\n    if m.len() != 8 {{ return 3; }}\n    let addr = m.address();\n    if addr == 0 {{ return 4; }}\n    let p = Pointer.fromAddress<Byte>(addr);\n    p.write(72);\n    let q1 = p + 1;\n    q1.write(101);\n    *(p + 2) = 108;\n    *(p + 3) = 108;\n    *(p + 4) = 111;\n    *(p + 5) = 33;\n    *(p + 6) = 33;\n    *(p + 7) = 33;\n    m.flush();\n    m.close();\n    if m.address() != 0 {{ return 5; }}\n    let ro: Result<Mmap, String> = fs.mmap(base + \"/seg.bin\", .Read);\n    if ro.isErr() {{ return 6; }}\n    let r: Mmap = ro.unwrap();\n    if r.len() != 8 {{ return 7; }}\n    let rp = Pointer.fromAddress<Byte>(r.address());\n    if rp.read() != 72 {{ return 8; }}\n    let rlast = rp + 7;\n    if rlast.read() != 33 {{ return 9; }}\n    r.close();\n    return 0;\n}}\n\nunsafe fn anonPart(): Int {{\n    if fs.mmapAnon(0).isOk() {{ return 20; }}\n    if fs.mmapAnon(0 - 4).isOk() {{ return 21; }}\n    let anon: Result<Mmap, String> = fs.mmapAnon(8);\n    if anon.isErr() {{ return 22; }}\n    let a: Mmap = anon.unwrap();\n    if a.len() != 8 {{ return 23; }}\n    let q = Pointer.fromAddress<Byte>(a.address());\n    q.write(65);\n    let q7 = q + 7;\n    q7.write(66);\n    if q.read() != 65 {{ return 24; }}\n    if q7.read() != 66 {{ return 25; }}\n    a.flush();\n    a.close();\n    return 0;\n}}\n\nfn Main(): Int {{\n    let base = \"{d}\";\n    fs.removeAll(base + \"/tree\");\n    if fs.mkdirAll(base + \"/tree\").isErr() {{ return 30; }}\n    if fs.writeText(base + \"/tree/empty.bin\", \"\", .Overwrite).isErr() {{ return 31; }}\n    let empty: Result<Mmap, String> = fs.mmapAnon(0);\n    if empty.isOk() {{ return 32; }}\n    unsafe {{\n        empty = fs.mmap(base + \"/tree/empty.bin\", .ReadWrite);\n    }}\n    if empty.isOk() {{ return 33; }}\n    let safeAnon: Result<Mmap, String> = fs.mmapAnon(16);\n    if safeAnon.isErr() {{ return 34; }}\n    let sa: Mmap = safeAnon.unwrap();\n    if sa.len() != 16 {{ return 35; }}\n    sa.flush();\n    sa.close();\n    if fs.mmapAnon(0).isOk() {{ return 36; }}\n    if fs.writeText(base + \"/seg.bin\", \"xxxxxxxx\", .Overwrite).isErr() {{ return 37; }}\n    let code = 0;\n    unsafe {{\n        code = filePart(base);\n    }}\n    if code != 0 {{ return 40 + code; }}\n    unsafe {{\n        code = anonPart();\n    }}\n    if code != 0 {{ return 60 + code; }}\n    let back: Result<String, String> = fs.readText(base + \"/seg.bin\");\n    if back.isErr() {{ return 50; }}\n    if back.unwrap() != \"Hello!!!\" {{ return 51; }}\n    print(\"@std/fs mmap verification success\");\n    return 0;\n}}\n"
    )
}

#[test]
fn test_fs_mmap_all_backends() {
    let dir = std::env::temp_dir().join(format!("rnx-fs-e2e-mmap-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src = mmap_src(&dir);
    check_all_backends(
        &src,
        0,
        &["@std/fs mmap verification success".to_string()],
        "fsmmap",
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_fs_mmap_needs_unsafe() {
    let err = lower_err(
        "import fs from \"@std/fs\";\n\nfn Main(): Int {\n    let m = fs.mmap(\"x.bin\", .ReadWrite);\n    return 0;\n}\n",
        "mmapcall",
    );
    assert_eq!(err.code, diagnostics::Code::E201, "{err:?}");
    let err = lower_err(
        "import fs, { Mmap } from \"@std/fs\";\n\nfn Main(): Int {\n    let m = new Mmap(0, 0);\n    let a = m.address();\n    return a;\n}\n",
        "mmapaddr",
    );
    assert_eq!(err.code, diagnostics::Code::E201, "{err:?}");
}

#[test]
fn test_fs_mmap_sigbus_doc() {
    let src = include_str!("../../stdlib/src/fs.rnx");
    assert!(src.contains("SIGBUS"), "fs.rnx must state the SIGBUS contract");
    assert!(
        src.contains("never truncate a mapped file"),
        "fs.rnx must state the truncation rule"
    );
    assert!(
        src.contains("mmap is not supported on this target"),
        "fs.rnx must state the WebAssembly rejection"
    );
}

#[test]
fn test_fs_mmap_s301_denial() {
    let dir = std::env::temp_dir().join(format!("rnx-fs-e2e-mmaps301-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    let target = dir.join("Project.config");
    let t = target.to_string_lossy();
    std::fs::write(
        &main,
        format!(
            "import fs, {{ Mmap }} from \"@std/fs\";\n\nunsafe fn grab(): Int {{\n    let m = fs.mmap(\"{t}\", .ReadWrite);\n    return 0;\n}}\n\nfn Main(): Int {{\n    let code = 0;\n    unsafe {{\n        code = grab();\n    }}\n    return code;\n}}\n"
        ),
    )
    .unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(&main)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = String::from_utf8_lossy(&build.stdout).lines().rev().find_map(|l| l.strip_prefix("artifact: ")).map(|s| s.trim().trim_end_matches(" (fresh)").to_string()).expect("build prints artifact path");
    let run = std::process::Command::new(&bin).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 1, "s301 denial exit");
    let stderr = String::from_utf8(run.stderr).unwrap();
    assert!(stderr.contains("S301"), "s301 denial message: {stderr}");
    assert!(!target.exists(), "protected file untouched");
    let _ = std::fs::remove_dir_all(&dir);
}
