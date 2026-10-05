use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

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

fn basics_src() -> String {
    r#"import io, { ColorLevel } from "@std/io";
import { Env } from "@std/env";
import { ByteBuffer } from "@std/bytes";

fn Main(): Int {
    io.write("line-one");
    io.write(42);
    io.write(true);
    io.writeRaw(ByteBuffer.fromString("AB"));
    io.writeError("err-line");
    print("line-one");
    print("a", 1, true);
    print(io.width() > 0);
    print(io.height() > 0);
    print(io.isTTY() == io.isTTY());
    print(io.width() == io.width());
    print(io.height() == io.height());
    let oldNoColor = Env.get("NO_COLOR");
    let oldColorTerm = Env.get("COLORTERM");
    Env.set("NO_COLOR", "1");
    if !(io.colorProfile() == ColorLevel.Ascii) {
        return 13;
    }
    print("profile-ascii");
    Env.set("NO_COLOR", oldNoColor);
    Env.set("COLORTERM", "truecolor");
    let prof = io.colorProfile();
    if io.isTTY() {
        if !(prof == ColorLevel.TrueColor) {
            return 15;
        }
    } else {
        if !(prof == ColorLevel.Ansi16) {
            return 16;
        }
    }
    print("profile-cap");
    Env.set("COLORTERM", oldColorTerm);
    let r1 = io.setRawMode(true);
    let r2 = io.setRawMode(false);
    print(r1.isOk() || r1.isErr());
    print(r1.isErr() == r2.isErr());
    if r1.isOk() {
        if !(r1.unwrapOr(false) == true) {
            return 22;
        }
        if !(r2.unwrapOr(false) == true) {
            return 23;
        }
    }
    io.clear();
    print("basics-ok");
    return 0;
}
"#
    .to_string()
}

/// Byte-faithful stdout: the interpreter captures raw output chunks, so
/// joining them reproduces the exact byte stream a native binary emits.
/// Assertions split or compare that stream instead of line entries.
fn stdout_text(chunks: &[String]) -> String {
    chunks.join("")
}

#[test]
fn std_io_basics_all_backends() {
    let src = basics_src();
    let want = "line-one\n42\ntrue\nABline-one\na 1 true\ntrue\ntrue\ntrue\ntrue\ntrue\nprofile-ascii\nprofile-cap\ntrue\ntrue\nbasics-ok\n";
    let (module, dir) = resolve_src(&src, "iobasics");
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(module));
    let mut machine = runtime::machine::Machine::new(leaked);
    let r = machine.call("Main", vec![]).unwrap_or_else(|e| panic!("interpreter iobasics: {e:?}"));
    match r {
        runtime::value::Value::Int(v) if v == 0 => {}
        other => panic!("interpreter iobasics: {other:?}"),
    }
    assert_eq!(stdout_text(&machine.output), want, "interpreter iobasics stdout");

    let mut jit = cranelift::jit::Jit::compile(leaked).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 0, "cranelift iobasics");
    assert_eq!(llvm::codegen::execute(leaked, "Main").unwrap(), 0, "llvm iobasics");

    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(dir.join("main.rnx"))
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = String::from_utf8_lossy(&build.stdout).lines().rev().find_map(|l| l.strip_prefix("artifact: ")).map(|s| s.trim().trim_end_matches(" (fresh)").to_string()).expect("build prints artifact path");
    let run = std::process::Command::new(&bin).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 0, "aot iobasics exit");
    assert_eq!(String::from_utf8(run.stdout).unwrap(), want, "aot iobasics stdout");
    let _ = std::fs::remove_dir_all(&dir);
}

fn piped_src() -> String {
    r#"import io, { ColorLevel } from "@std/io";
import { Env } from "@std/env";

fn Main(): Int {
    let line = io.readLine();
    if line.isErr() {
        return 1;
    }
    if !(line.unwrapOr("") == "hello") {
        return 2;
    }
    let empty = io.readLine();
    if empty.isErr() {
        return 3;
    }
    if !(empty.unwrapOr("?") == "") {
        return 4;
    }
    let rest = io.read();
    if rest.isErr() {
        return 5;
    }
    if !(rest.unwrapOr("") == "tail") {
        return 6;
    }
    let eof = io.readLine();
    if eof.isOk() {
        return 7;
    }
    io.write("done");
    io.writeError("edone");
    if io.isTTY() {
        return 8;
    }
    if io.width() != 80 {
        return 9;
    }
    if io.height() != 24 {
        return 10;
    }
    Env.set("NO_COLOR", "1");
    if !(io.colorProfile() == ColorLevel.Ascii) {
        return 11;
    }
    let r1 = io.setRawMode(true);
    let r2 = io.setRawMode(false);
    if r1.isOk() || r2.isOk() {
        return 12;
    }
    print("piped-ok");
    return 0;
}
"#
    .to_string()
}

fn run_piped_capture(tag: &str, prog: &str, input: &[u8]) -> (i32, String, String) {
    let mut child = Command::new(prog)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn {tag}: {e}"));
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(input)
        .unwrap_or_else(|e| panic!("stdin {tag}: {e}"));
    let out = child.wait_with_output().unwrap_or_else(|e| panic!("wait {tag}: {e}"));
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

#[test]
fn std_io_piped_stdin_stdout() {
    let dir = std::env::temp_dir().join(format!("rnx-iopiped-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, piped_src()).unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");

    let run = Command::new(rnx)
        .arg("run")
        .arg(&main)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let input = b"hello\n\ntail";
    let mut run = run;
    run.stdin.as_mut().unwrap().write_all(input).unwrap();
    let out = run.wait_with_output().unwrap();
    assert_eq!(out.status.code().unwrap(), 0, "run {}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "done\npiped-ok\n", "run stdout");
    assert_eq!(String::from_utf8(out.stderr).unwrap(), "edone\n", "run stderr");

    let build = Command::new(rnx).arg("build").arg(&main).output().unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = String::from_utf8_lossy(&build.stdout).lines().rev().find_map(|l| l.strip_prefix("artifact: ")).map(|s| s.trim().trim_end_matches(" (fresh)").to_string()).expect("build prints artifact path");
    let (code, stdout, stderr) = run_piped_capture("aot", &bin, input);
    assert_eq!(code, 0, "aot exit, stderr was {stderr}");
    assert_eq!(stdout, "done\npiped-ok\n", "aot stdout");
    assert_eq!(stderr, "edone\n", "aot stderr");
    let _ = std::fs::remove_dir_all(&dir);
}
