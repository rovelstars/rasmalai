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

fn fixture_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-fs-e2e-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("zeta.txt"), "z").unwrap();
    std::fs::write(dir.join("a.txt"), "hello").unwrap();
    std::fs::write(dir.join("sub").join("c.rnx"), "c").unwrap();
    std::fs::write(dir.join("sub").join("b.rnx"), "b").unwrap();
    dir
}

fn readonly_src(dir: &PathBuf) -> String {
    let d = dir.to_string_lossy();
    format!(
        "import fs, {{ FileStat }} from \"@std/fs\";\n\nfn Main(): Int {{\n    if !fs.exists(\"{d}/a.txt\") {{ return 1; }}\n    if !fs.isFile(\"{d}/a.txt\") {{ return 2; }}\n    if fs.isDir(\"{d}/a.txt\") {{ return 3; }}\n    if !fs.isDir(\"{d}/sub\") {{ return 4; }}\n    if fs.exists(\"{d}/missing.txt\") {{ return 5; }}\n    if fs.isFile(\"{d}/missing.txt\") {{ return 6; }}\n    if fs.isDir(\"{d}/missing.txt\") {{ return 7; }}\n    let st = fs.stat(\"{d}/a.txt\");\n    if st.isErr() {{ return 8; }}\n    let s: FileStat = st.unwrap();\n    if s.size != 5 {{ return 9; }}\n    if !s.isFile {{ return 10; }}\n    if s.isDir {{ return 11; }}\n    if s.modified <= 0 {{ return 12; }}\n    if fs.stat(\"{d}/missing.txt\").isOk() {{ return 13; }}\n    let rd = fs.readDir(\"{d}\");\n    if rd.isErr() {{ return 14; }}\n    let names: Array<String> = rd.unwrap();\n    if names.len() != 3 {{ return 15; }}\n    if names[0] != \"a.txt\" {{ return 16; }}\n    if names[1] != \"sub\" {{ return 17; }}\n    if names[2] != \"zeta.txt\" {{ return 18; }}\n    if fs.readDir(\"{d}/missing-dir\").isOk() {{ return 19; }}\n    let hits: Array<String> = fs.glob(\"{d}/*.txt\").unwrap();\n    if hits.len() != 2 {{ return 20; }}\n    if hits[0] != \"{d}/a.txt\" {{ return 21; }}\n    if hits[1] != \"{d}/zeta.txt\" {{ return 22; }}\n    let deep: Array<String> = fs.glob(\"{d}/**/*.rnx\").unwrap();\n    if deep.len() != 2 {{ return 23; }}\n    if deep[0] != \"{d}/sub/b.rnx\" {{ return 24; }}\n    if deep[1] != \"{d}/sub/c.rnx\" {{ return 25; }}\n    if fs.glob(\"\").isOk() {{ return 26; }}\n    if fs.readLink(\"{d}/missing.txt\").isOk() {{ return 27; }}\n    if fs.readLink(\"{d}/a.txt\").isOk() {{ return 28; }}\n    print(\"@std/fs readonly verification success\");\n    return 0;\n}}\n"
    )
}

#[test]
fn test_fs_readonly_all_backends() {
    let dir = fixture_dir("ro");
    let src = readonly_src(&dir);
    check_all_backends(
        &src,
        0,
        &["@std/fs readonly verification success".to_string()],
        "fsreadonly",
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn test_fs_readlink_all_backends() {
    use std::os::unix::fs::symlink;
    let dir = fixture_dir("link");
    let target = dir.join("a.txt").to_string_lossy().into_owned();
    symlink(&target, dir.join("link.txt")).unwrap();
    let d = dir.to_string_lossy();
    let src = format!(
        "import fs, {{ FileStat }} from \"@std/fs\";\n\nfn Main(): Int {{\n    let link = fs.readLink(\"{d}/link.txt\");\n    if link.isErr() {{ return 1; }}\n    if link.unwrap() != \"{target}\" {{ return 2; }}\n    let st = fs.stat(\"{d}/link.txt\");\n    if st.isErr() {{ return 3; }}\n    let s: FileStat = st.unwrap();\n    if !s.isLink {{ return 4; }}\n    if s.isFile {{ return 5; }}\n    print(\"@std/fs readlink verification success\");\n    return 0;\n}}\n"
    );
    check_all_backends(
        &src,
        0,
        &["@std/fs readlink verification success".to_string()],
        "fsreadlink",
    );
    let _ = std::fs::remove_dir_all(&dir);
}
