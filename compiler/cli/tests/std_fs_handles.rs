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

fn handles_src(dir: &PathBuf) -> String {
    let d = dir.to_string_lossy();
    format!(
        "import fs, {{ File }} from \"@std/fs\";\n\nfn Main(): Int {{\n    let base = \"{d}\";\n    fs.removeAll(base + \"/handles\");\n    if fs.mkdirAll(base + \"/handles\").isErr() {{ return 1; }}\n    let w = File.create(base + \"/handles/a.txt\", .Overwrite);\n    if !w.isOpen {{ return 2; }}\n    if w.writeText(\"0123456789\").unwrapOr(0 - 1) != 10 {{ return 3; }}\n    if w.tell() != 10 {{ return 4; }}\n    if w.seek(4).unwrapOr(0 - 1) != 4 {{ return 5; }}\n    if w.tell() != 4 {{ return 6; }}\n    if w.len().unwrapOr(0 - 1) != 10 {{ return 7; }}\n    if w.sync().isErr() {{ return 8; }}\n    if w.truncate(3).isErr() {{ return 9; }}\n    if w.len().unwrapOr(0 - 1) != 3 {{ return 10; }}\n    w.flush();\n    w.close();\n    let r = File.open(base + \"/handles/a.txt\", .Read);\n    if !r.isOpen {{ return 11; }}\n    if r.readText().unwrapOr(\"ERR\") != \"012\" {{ return 12; }}\n    if r.tell() != 3 {{ return 13; }}\n    if r.seek(1).unwrapOr(0 - 1) != 1 {{ return 14; }}\n    if r.readText().unwrapOr(\"ERR\") != \"12\" {{ return 15; }}\n    r.close();\n    if r.readText().isOk() {{ return 16; }}\n    if r.tell() != 0 - 1 {{ return 17; }}\n    if r.seek(0).isOk() {{ return 18; }}\n    if r.len().isOk() {{ return 19; }}\n    if r.sync().isOk() {{ return 20; }}\n    if r.truncate(1).isOk() {{ return 21; }}\n    if r.writeText(\"x\").isOk() {{ return 22; }}\n    let missing = File.open(base + \"/handles/nope.txt\", .Read);\n    if missing.isOpen {{ return 23; }}\n    let dup = File.create(base + \"/handles/a.txt\", .CreateNew);\n    if dup.isOpen {{ return 24; }}\n    let kept = File.create(base + \"/handles/a.txt\", .Create);\n    if !kept.isOpen {{ return 25; }}\n    if kept.writeText(\"AB\").unwrapOr(0 - 1) != 2 {{ return 26; }}\n    kept.close();\n    if fs.readText(base + \"/handles/a.txt\").unwrapOr(\"\") != \"AB2\" {{ return 27; }}\n    print(\"@std/fs handles verification success\");\n    return 0;\n}}\n"
    )
}

#[test]
fn std_fs_handle_text_seek_len() {
    let dir = std::env::temp_dir().join(format!("rnx-fshandles-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src = handles_src(&dir);
    check_all_backends(&src, 0, &["@std/fs handles verification success".to_string()], "fshandles");
    let _ = std::fs::remove_dir_all(&dir);
}

fn bytes_src(dir: &PathBuf) -> String {
    let d = dir.to_string_lossy();
    format!(
        "import fs, {{ File }} from \"@std/fs\";\nimport {{ ByteBuffer }} from \"@std/bytes\";\n\nfn Main(): Int {{\n    let base = \"{d}\";\n    fs.removeAll(base + \"/blobs\");\n    if fs.mkdirAll(base + \"/blobs\").isErr() {{ return 1; }}\n    let out = ByteBuffer.allocate(8);\n    out.writeInt32BE(0, 0x12345678);\n    out.writeInt32BE(4, 42);\n    let bw = File.create(base + \"/blobs/b.bin\", .Overwrite);\n    if !bw.isOpen {{ return 2; }}\n    if bw.writeBytes(out, 0, 8).unwrapOr(0 - 1) != 8 {{ return 3; }}\n    bw.close();\n    let br = File.open(base + \"/blobs/b.bin\", .Read);\n    if !br.isOpen {{ return 4; }}\n    let back = ByteBuffer.allocate(8);\n    if br.readBytes(back, 0, 8).unwrapOr(0 - 1) != 8 {{ return 5; }}\n    br.close();\n    if back.readInt32BE(0) != 0x12345678 {{ return 6; }}\n    if back.readInt32BE(4) != 42 {{ return 7; }}\n    if br.readBytes(back, 0, 1).isOk() {{ return 8; }}\n    print(\"@std/fs handle bytes verification success\");\n    return 0;\n}}\n"
    )
}

#[test]
fn std_fs_handle_bytes_roundtrip() {
    let dir = std::env::temp_dir().join(format!("rnx-fsbytes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src = bytes_src(&dir);
    check_all_backends(&src, 0, &["@std/fs handle bytes verification success".to_string()], "fsbytes");
    let _ = std::fs::remove_dir_all(&dir);
}

fn path_src(dir: &PathBuf) -> String {
    let d = dir.to_string_lossy();
    format!(
        "import {{ Path }} from \"@std/fs\";\n\nfn Main(): Int {{\n    if Path.join(\"libs\", \"econ\") != \"libs/econ\" {{ return 1; }}\n    if Path.join(\"\", \"b\") != \"b\" {{ return 2; }}\n    if Path.join(\"a\", \"\") != \"a\" {{ return 3; }}\n    if Path.dir(\"a/b/c.txt\") != \"a/b\" {{ return 4; }}\n    if Path.dir(\"c.txt\") != \".\" {{ return 5; }}\n    if Path.dir(\"/a\") != \"/\" {{ return 6; }}\n    if Path.dir(\"/\") != \"/\" {{ return 7; }}\n    if Path.dir(\"\") != \".\" {{ return 8; }}\n    if Path.dir(\"a/b/\") != \"a\" {{ return 9; }}\n    if Path.base(\"a/b/c.txt\") != \"c.txt\" {{ return 10; }}\n    if Path.base(\"a/b/\") != \"b\" {{ return 11; }}\n    if Path.base(\"/\") != \"/\" {{ return 12; }}\n    if Path.base(\"\") != \"\" {{ return 13; }}\n    if Path.ext(\"a/b.txt\") != \".txt\" {{ return 14; }}\n    if Path.ext(\"archive.tar.gz\") != \".gz\" {{ return 15; }}\n    if Path.ext(\"noext\") != \"\" {{ return 16; }}\n    if Path.ext(\".bashrc\") != \"\" {{ return 17; }}\n    if !Path.isAbs(\"/tmp/x\") {{ return 18; }}\n    if Path.isAbs(\"rel/x\") {{ return 19; }}\n    if Path.isAbs(\"\") {{ return 20; }}\n    if !Path.exists(\"{d}\") {{ return 21; }}\n    if !Path.isDir(\"{d}\") {{ return 22; }}\n    if Path.isFile(\"{d}\") {{ return 23; }}\n    if Path.exists(\"{d}/definitely-missing\") {{ return 24; }}\n    print(\"@std/fs path verification success\");\n    return 0;\n}}\n"
    )
}

#[test]
fn std_fs_path_helpers() {
    let dir = std::env::temp_dir().join(format!("rnx-fspath-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src = path_src(&dir);
    check_all_backends(&src, 0, &["@std/fs path verification success".to_string()], "fspath");
    let _ = std::fs::remove_dir_all(&dir);
}

fn lock_src(dir: &PathBuf) -> String {
    let d = dir.to_string_lossy();
    format!(
        "import fs, {{ File }} from \"@std/fs\";\nimport {{ AtomicInt, Channel }} from \"@std/sync\";\n\nfn locker() {{\n    let f = File.open(\"{d}/lock.txt\", .ReadWrite);\n    f.lockSync();\n    Channel.byId(961).send(1);\n    Channel.byId(962).recv();\n    f.unlockSync();\n    f.close();\n}}\n\nfn tryer() {{\n    let f = File.open(\"{d}/lock.txt\", .ReadWrite);\n    if f.tryLockSync() {{\n        AtomicInt.byId(951).set(1);\n        f.unlockSync();\n    }} else {{\n        AtomicInt.byId(951).set(0);\n    }}\n    f.close();\n}}\n\nfn Main(): Int {{\n    fs.writeText(\"{d}/lock.txt\", \"x\", .Overwrite);\n    AtomicInt.byId(951).set(0 - 1);\n    let pool1 = ThreadPool.byId(951, 1);\n    pool1.submit(locker);\n    Channel.byId(961).recv();\n    let pool2 = ThreadPool.byId(952, 1);\n    pool2.submit(tryer);\n    pool2.join();\n    pool2.shutdown();\n    if AtomicInt.byId(951).get() != 0 {{ return 1; }}\n    Channel.byId(962).send(1);\n    pool1.join();\n    pool1.shutdown();\n    let f = File.open(\"{d}/lock.txt\", .ReadWrite);\n    if !f.isOpen {{ return 2; }}\n    if !f.tryLockSync() {{ return 3; }}\n    f.unlockSync();\n    f.close();\n    print(\"@std/fs lock verification success\");\n    return 0;\n}}\n"
    )
}

#[test]
fn std_fs_file_lock_exclusivity() {
    let dir = std::env::temp_dir().join(format!("rnx-fslock-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src = lock_src(&dir);
    check_all_backends(&src, 0, &["@std/fs lock verification success".to_string()], "fslock");
    let _ = std::fs::remove_dir_all(&dir);
}
