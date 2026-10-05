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

const ASYNC_SRC: &str = r#"import fs, { FileStat } from "@std/fs";
import { Result, Promise } from "@std/prelude";
import { ByteBuffer } from "@std/bytes";

async fn loadText(path: String): String {
    let p: Promise<Result<String, String>> = fs.readTextAsync(path);
    let r: Result<String, String> = await p;
    return r.unwrapOr("FALLBACK");
}

fn Main(): Int {
    let base = "__BASE__";
    fs.removeAll(base + "/async");
    if fs.mkdirAll(base + "/async").isErr() { return 1; }
    if fs.setWorkers(0) != 1 { return 2; }
    if fs.setWorkers(99) != 8 { return 3; }
    if fs.setWorkers(4) != 4 { return 4; }
    if fs.writeText(base + "/async/a.txt", "hello-async", .Overwrite).isErr() { return 5; }
    let t: Result<String, String> = fs.readTextAsync(base + "/async/a.txt").wait().unwrap();
    if t.isErr() { return 6; }
    if t.unwrap() != "hello-async" { return 7; }
    let g: Result<String, String> = loadText(base + "/async/a.txt").wait();
    if g.isErr() { return 8; }
    if g.unwrap() != "hello-async" { return 9; }
    let e: Result<String, String> = fs.readTextAsync(base + "/async/missing.txt").wait().unwrap();
    if e.isOk() { return 10; }
    let w: Result<Int, String> = fs.writeTextAsync(base + "/async/b.txt", "wval", .Overwrite).wait().unwrap();
    if w.isErr() { return 11; }
    if fs.readText(base + "/async/b.txt").unwrap() != "wval" { return 12; }
    let bb = ByteBuffer.fromArray([9, 8, 7, 6]);
    let wb: Result<Int, String> = fs.writeBytesAsync(base + "/async/c.bin", bb, .Overwrite).wait().unwrap();
    if wb.isErr() { return 13; }
    let rb: Result<ByteBuffer, String> = fs.readBytesAsync(base + "/async/c.bin").wait().unwrap();
    if rb.isErr() { return 14; }
    let got: ByteBuffer = rb.unwrap();
    if got.length() != 4 { return 15; }
    if got.readUInt8(3) != 6 { return 16; }
    let st: Result<FileStat, String> = fs.statAsync(base + "/async/a.txt").wait().unwrap();
    if st.isErr() { return 17; }
    let fstat: FileStat = st.unwrap();
    if fstat.size != 11 { return 18; }
    let rd: Result<Array<String>, String> = fs.readDirAsync(base + "/async").wait().unwrap();
    if rd.isErr() { return 19; }
    let names: Array<String> = rd.unwrap();
    if names.len() != 3 { return 20; }
    let gl: Result<Array<String>, String> = fs.globAsync(base + "/async/*.txt").wait().unwrap();
    if gl.isErr() { return 21; }
    let hits: Array<String> = gl.unwrap();
    if hits.len() != 2 { return 22; }
    let ca: Result<Bool, String> = fs.copyAsync(base + "/async/a.txt", base + "/async/cp.txt", .Overwrite).wait().unwrap();
    if ca.isErr() { return 23; }
    let ma: Result<Bool, String> = fs.moveAsync(base + "/async/cp.txt", base + "/async/mv.txt").wait().unwrap();
    if ma.isErr() { return 24; }
    let ra: Result<Bool, String> = fs.renameAsync(base + "/async/mv.txt", base + "/async/rn.txt").wait().unwrap();
    if ra.isErr() { return 25; }
    if fs.readText(base + "/async/rn.txt").unwrap() != "hello-async" { return 26; }
    let ps: Array<Promise<Result<String, String>>> = [];
    let i = 0;
    while i < 8 {
        ps.push(fs.readTextAsync(base + "/async/a.txt"));
        i = i + 1;
    }
    let j = 0;
    while j < 8 {
        let r: Result<String, String> = ps[j].wait().unwrap();
        if r.isErr() { return 27; }
        if r.unwrap() != "hello-async" { return 28; }
        j = j + 1;
    }
    let p: Promise<Result<String, String>> = fs.readTextAsync(base + "/async/a.txt");
    let chained: Promise<String> = p.then<String>((r: Result<String, String>): String => {
        let inner: Result<String, String> = fs.readTextAsync(base + "/async/b.txt").wait().unwrap();
        return inner.unwrapOr("nil") + "|" + r.unwrapOr("nil");
    });
    let out: Result<String, String> = chained.wait();
    if out.isErr() { return 29; }
    let done: String = out.unwrap();
    if done != "wval|hello-async" { return 30; }
    print("@std/fs async verification success");
    return 0;
}
"#;

#[test]
fn test_fs_async_all_backends() {
    let dir = std::env::temp_dir().join(format!("rnx-fs-e2e-async-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src = ASYNC_SRC.replace("__BASE__", &dir.to_string_lossy());
    check_all_backends(
        &src,
        0,
        &["@std/fs async verification success".to_string()],
        "fsasync",
    );
    let _ = std::fs::remove_dir_all(&dir);
}
