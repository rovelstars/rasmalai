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
    let want_text = if want_out.is_empty() { String::new() } else { want_out.join("\n") + "\n" };
    assert_eq!(stdout_text(&machine.output), want_text, "interpreter {tag} stdout");

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
    assert_eq!(String::from_utf8(run.stdout).unwrap(), want_text, "aot {tag} stdout");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Byte-faithful stdout: the interpreter captures raw output chunks, so
/// joining them reproduces the exact byte stream a native binary emits.
/// Assertions compare that stream instead of line entries.
fn stdout_text(chunks: &[String]) -> String {
    chunks.join("")
}

fn pretty_src() -> String {
    r#"import io from "@std/io";
import { Map, Set } from "@std/collections";

class Point {
    let x: Int;
    let y: Int;
    init(x: Int, y: Int) {
        this.x = x;
        this.y = y;
    }
}

fn Main(): Int {
    print("hi", 42, true);
    io.write(2.0);
    io.write(null);
    io.write([1, 2, 3]);
    print([1, 2], "x");
    io.write([[1, 2], [3]]);
    let mixed: Array<Any> = [];
    mixed.push(1);
    mixed.push("two");
    mixed.push(true);
    io.write(mixed);
    let deep = [[[[1]]]];
    io.write(deep);
    let m = new Map<String, Int>();
    m.set("a", 1);
    m.set("b", 2);
    io.write(m);
    let r = Result.Ok(7);
    io.write(r);
    let e = Result.Err("boom");
    io.write(e);
    let p = new Point(1, 2);
    io.write(p);
    print(p);
    let nested = new Map<String, Any>();
    nested.set("p", p);
    nested.set("n", 5);
    io.write(nested);
    let s = new Set<String>();
    s.add("ore");
    s.add("coal");
    io.write(s);
    let c: Array<Any> = [];
    c.push(c);
    io.write(c);
    return 0;
}
"#
    .to_string()
}

#[test]
fn std_io_pretty_all_backends() {
    let want_out: Vec<String> = vec![
        "hi 42 true",
        "2.0",
        "null",
        "[1, 2, 3]",
        "[1, 2] x",
        "[[1, 2], [3]]",
        "[1, two, true]",
        "[[[...]]]",
        "{a: 1, b: 2}",
        "Ok(7)",
        "Err(boom)",
        "Point{x: 1, y: 2}",
        "Point{x: 1, y: 2}",
        "{p: Point{x: 1, y: 2}, n: 5}",
        "[ore, coal]",
        "[<cycle>]",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    check_all_backends(&pretty_src(), 0, &want_out, "iopretty");
}

#[test]
fn std_io_pretty_enum_cycle_terminates() {
    let src = r#"import io from "@std/io";
enum E {
    V(Any)
}
fn Main(): Int {
    let c: Array<Any> = [];
    let e = E.V(c);
    c.push(e);
    io.write(e);
    return 0;
}
"#
    .to_string();
    check_all_backends(&src, 0, &["V([<cycle>])".to_string()], "ioprettycycle");
}

fn err_src() -> String {    r#"import io from "@std/io";

fn Main(): Int {
    io.writeError("boom");
    io.writeError([1, 2]);
    return 0;
}
"#
    .to_string()
}

#[test]
fn std_io_pretty_piped_no_escapes() {
    let dir = std::env::temp_dir().join(format!("rnx-iopretty2-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, err_src()).unwrap();
    std::fs::write(&dir.join("pretty.rnx"), pretty_src()).unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");

    let bin_path = dir.join("pretty_bin");
    let build = Command::new(rnx)
        .arg("build")
        .arg(dir.join("pretty.rnx"))
        .arg("-o")
        .arg(&bin_path)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let mut child = Command::new(&bin_path)
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.as_mut().unwrap().write_all(b"").unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code().unwrap(), 0, "aot exit, stderr was {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(!stdout.contains('\x1b'), "no ESC bytes when piped, got {stdout:?}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(first, "hi 42 true", "scalars identical, got {first:?}");

    let err_bin = dir.join("err_bin");
    let build = Command::new(rnx)
        .arg("build")
        .arg(&main)
        .arg("-o")
        .arg(&err_bin)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let out = Command::new(&err_bin).output().unwrap();
    assert_eq!(out.status.code().unwrap(), 0);
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert_eq!(stderr, "boom\n[1, 2]\n", "writeError(stderr) got {stderr:?}");
    assert!(!stderr.contains('\x1b'), "no ESC bytes on piped stderr");
    let _ = std::fs::remove_dir_all(&dir);
}

fn color_src() -> String {
    r#"import io from "@std/io";

fn Main(): Int {
    io.write([1, 2]);
    return 0;
}
"#
    .to_string()
}

#[test]
fn std_io_pretty_tty_color() {
    let dir = std::env::temp_dir().join(format!("rnx-iopretty3-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, color_src()).unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let bin_path = dir.join("color_bin");
    let build = Command::new(rnx)
        .arg("build")
        .arg(&main)
        .arg("-o")
        .arg(&bin_path)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));

    let script = Command::new("script")
        .args(["-qec", bin_path.to_str().unwrap(), "/dev/null"])
        .stdin(Stdio::null())
        .env_remove("NO_COLOR")
        .env("TERM", "xterm")
        .env_remove("COLORTERM")
        .output();
    let out = match script {
        Ok(o) => o,
        Err(e) => {
            eprintln!("skip tty color test: no `script` helper ({e})");
            return;
        }
    };
    if !out.status.success() {
        eprintln!("skip tty color test: `script` failed");
        return;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains('\x1b'), "ESC bytes on a TTY, got {stdout:?}");

    let out = Command::new("script")
        .args(["-qec", bin_path.to_str().unwrap(), "/dev/null"])
        .stdin(Stdio::null())
        .env("NO_COLOR", "1")
        .env("TERM", "xterm")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains('\x1b'), "NO_COLOR strips escapes on a TTY, got {stdout:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
