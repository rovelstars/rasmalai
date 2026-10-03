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

    let bin_path = dir.join("arc_bin");
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

fn write_src(dir: &PathBuf) -> String {
    let d = dir.to_string_lossy();
    format!(
        "import fs, {{ FileStat }} from \"@std/fs\";\nimport {{ Result }} from \"@std/prelude\";\nimport {{ ByteBuffer }} from \"@std/bytes\";\n\nfn Main(): Int {{\n    let base = \"{d}\";\n    fs.removeAll(base + \"/tree\");\n    if fs.mkdirAll(base + \"/tree/a/b\").isErr() {{ return 1; }}\n    if fs.mkdir(base + \"/tree/one\").isErr() {{ return 2; }}\n    if fs.mkdir(base + \"/tree/no-parent/x\").isOk() {{ return 3; }}\n    let w: Result<Int, String> = fs.writeText(base + \"/tree/hello.txt\", \"hello\", .Overwrite);\n    if w.isErr() {{ return 4; }}\n    if w.unwrap() != 5 {{ return 5; }}\n    if fs.writeText(base + \"/tree/hello.txt\", \"hello\", .CreateNew).isOk() {{ return 6; }}\n    if fs.writeText(base + \"/tree/hello.txt\", \" world\", .Append).isErr() {{ return 7; }}\n    let t: Result<String, String> = fs.readText(base + \"/tree/hello.txt\");\n    if t.isErr() {{ return 8; }}\n    if t.unwrap() != \"hello world\" {{ return 9; }}\n    if fs.readText(base + \"/tree/missing.txt\").isOk() {{ return 10; }}\n    if fs.stat(base + \"/tree/missing.txt\").isOk() {{ return 11; }}\n    if fs.readDir(base + \"/tree/missing-dir\").isOk() {{ return 12; }}\n    let buf = ByteBuffer.fromArray([1, 2, 3, 250]);\n    if fs.writeBytes(base + \"/tree/blob.bin\", buf, .Overwrite).isErr() {{ return 13; }}\n    let back: Result<ByteBuffer, String> = fs.readBytes(base + \"/tree/blob.bin\");\n    if back.isErr() {{ return 14; }}\n    let bb: ByteBuffer = back.unwrap();\n    if bb.length() != 4 {{ return 15; }}\n    if bb.readUInt8(0) != 1 {{ return 16; }}\n    if bb.readUInt8(3) != 250 {{ return 17; }}\n    if fs.readBytes(base + \"/tree/missing.bin\").isOk() {{ return 18; }}\n    if fs.copy(base + \"/tree/hello.txt\", base + \"/tree/cp.txt\", .Overwrite).isErr() {{ return 19; }}\n    if fs.readText(base + \"/tree/cp.txt\").unwrap() != \"hello world\" {{ return 20; }}\n    if fs.copy(base + \"/tree/hello.txt\", base + \"/tree/cp.txt\", .SkipExisting).isErr() {{ return 21; }}\n    if fs.copy(base + \"/tree/missing.txt\", base + \"/tree/cp.txt\", .Overwrite).isOk() {{ return 22; }}\n    if fs.move(base + \"/tree/cp.txt\", base + \"/tree/mv.txt\").isErr() {{ return 23; }}\n    if fs.exists(base + \"/tree/cp.txt\") {{ return 24; }}\n    if fs.rename(base + \"/tree/mv.txt\", base + \"/tree/rn.txt\").isErr() {{ return 25; }}\n    if fs.rename(base + \"/tree/missing.txt\", base + \"/tree/rn.txt\").isOk() {{ return 26; }}\n    if fs.truncate(base + \"/tree/rn.txt\", 5).isErr() {{ return 27; }}\n    if fs.readText(base + \"/tree/rn.txt\").unwrap() != \"hello\" {{ return 28; }}\n    if fs.truncate(base + \"/tree/rn.txt\", 0 - 1).isOk() {{ return 29; }}\n    let st: FileStat = fs.stat(base + \"/tree/rn.txt\").unwrap();\n    if st.size != 5 {{ return 30; }}\n    if !st.isFile {{ return 31; }}\n    if fs.fsync(base + \"/tree/rn.txt\").isErr() {{ return 32; }}\n    if fs.fsync(base + \"/tree/missing.txt\").isOk() {{ return 33; }}\n    let rd: Array<String> = fs.readDir(base + \"/tree\").unwrap();\n    if rd.len() != 5 {{ return 34; }}\n    if rd[0] != \"a\" {{ return 35; }}\n    if fs.remove(base + \"/tree/rn.txt\").unwrap() != true {{ return 36; }}\n    if fs.remove(base + \"/tree/rn.txt\").unwrap() != false {{ return 37; }}\n    if fs.remove(base + \"/tree\").isOk() {{ return 38; }}\n    if fs.removeAll(base + \"/tree\").unwrap() != true {{ return 39; }}\n    if fs.removeAll(base + \"/tree\").unwrap() != false {{ return 40; }}\n    if fs.exists(base + \"/tree\") {{ return 41; }}\n    print(\"@std/fs write verification success\");\n    return 0;\n}}\n"
    )
}

#[test]
fn test_fs_write_all_backends() {
    let dir = std::env::temp_dir().join(format!("rnx-fs-e2e-write-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src = write_src(&dir);
    check_all_backends(
        &src,
        0,
        &["@std/fs write verification success".to_string()],
        "fswrite",
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn test_fs_symlink_chmod_all_backends() {
    let dir = std::env::temp_dir().join(format!("rnx-fs-e2e-link-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let d = dir.to_string_lossy();
    let src = format!(
        "import fs, {{ FileStat }} from \"@std/fs\";\nimport {{ Result }} from \"@std/prelude\";\n\nfn Main(): Int {{\n    let base = \"{d}\";\n    fs.removeAll(base + \"/real.txt\");\n    fs.removeAll(base + \"/link.txt\");\n    if fs.writeText(base + \"/real.txt\", \"data\", .Overwrite).isErr() {{ return 1; }}\n    if fs.symlink(base + \"/real.txt\", base + \"/link.txt\").isErr() {{ return 2; }}\n    let tgt: Result<String, String> = fs.readLink(base + \"/link.txt\");\n    if tgt.isErr() {{ return 3; }}\n    if tgt.unwrap() != base + \"/real.txt\" {{ return 4; }}\n    let st: FileStat = fs.stat(base + \"/link.txt\").unwrap();\n    if !st.isLink {{ return 5; }}\n    if st.isFile {{ return 6; }}\n    if fs.symlink(base + \"/real.txt\", base + \"/link.txt\").isOk() {{ return 7; }}\n    if fs.chmod(base + \"/real.txt\", 384).isErr() {{ return 8; }}\n    let back: FileStat = fs.stat(base + \"/real.txt\").unwrap();\n    if back.mode == 0 {{ return 9; }}\n    if fs.readText(base + \"/link.txt\").unwrap() != \"data\" {{ return 10; }}\n    print(\"@std/fs symlink verification success\");\n    return 0;\n}}\n"
    );
    check_all_backends(
        &src,
        0,
        &["@std/fs symlink verification success".to_string()],
        "fslink",
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_fs_s301_denial() {
    let dir = std::env::temp_dir().join(format!("rnx-fs-e2e-s301-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    let target = dir.join("Project.config");
    let t = target.to_string_lossy();
    std::fs::write(
        &main,
        format!(
            "import fs, {{ FileStat }} from \"@std/fs\";\n\nfn Main(): Int {{\n    fs.writeText(\"{t}\", \"evil\", .Overwrite);\n    return 0;\n}}\n"
        ),
    )
    .unwrap();
    let bin_path = dir.join("s301_bin");
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(&main)
        .arg("-o")
        .arg(&bin_path)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new(&bin_path).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 1, "s301 denial exit");
    let stderr = String::from_utf8(run.stderr).unwrap();
    assert!(stderr.contains("S301"), "s301 denial message: {stderr}");
    assert!(!target.exists(), "protected file untouched");
    let _ = std::fs::remove_dir_all(&dir);
}
