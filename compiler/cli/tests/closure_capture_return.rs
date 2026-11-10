#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::process::Command;

// Regression pin for closure captures that are returned to the caller.
//
// Failure mode was: a closure that returned a captured heap value handed the
// caller the environment's reference without retaining it. The call site
// treats every return as owned and releases it, so calling the closure twice
// dropped the last reference and aborted with
// `rnx panic: rnx_release on zero count` on Cranelift and LLVM.
//
// Fix: closure capture slots stay owned by the closure environment. When a
// return lowers to the capture slot itself, emit a retain so the caller's
// release balances and the environment still owns its copy. Named function
// parameters keep the move semantics where the caller transfers ownership.

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn run(dir: &std::path::Path, src: &str, backend: &str) -> String {
    let file = dir.join(format!("{backend}.rnx"));
    std::fs::write(&file, src).unwrap();
    let out = Command::new(rnx())
        .arg("run")
        .arg("--backend")
        .arg(backend)
        .arg(&file)
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "{backend} failed: {stderr}{stdout}");
    assert!(
        !stderr.contains("panicked") && !stderr.contains("rnx panic"),
        "{backend} panicked: {stderr}"
    );
    assert!(
        !stderr.contains("rnx_release on zero count")
            && !stdout.contains("rnx_release on zero count"),
        "{backend} over-released: {stderr}{stdout}"
    );
    stdout.trim().to_string()
}

const BUFFER_RETURN: &str = r#"import { ByteBuffer } from "@std/bytes"

fn call(f: fn(): ByteBuffer?): Int {
    let got: ByteBuffer? = f()
    if got == null { return -1 }
    let v: ByteBuffer = got
    return v.length()
}

fn Main(): Int {
    let data = ByteBuffer.fromString("hello")
    let f = (): ByteBuffer? => data
    let a: ByteBuffer? = f()
    let b: ByteBuffer? = f()
    if a == null || b == null { return 1 }
    let av: ByteBuffer = a
    let bv: ByteBuffer = b
    println!(av.length(), bv.length(), call(f), call(f))
    let after = ByteBuffer.fromString(data.readString(0, data.length()))
    println!(after.length())
    return 0
}
"#;

const STRING_AND_OBJECT_RETURN: &str = r#"class Box {
    let name: String
    init(name: String) { this.name = name }
}

fn Main(): Int {
    let s = "captured"
    let g = (): String => s
    let box = new Box("boxed")
    let h = (): Box => box
    let x: Box = h()
    let y: Box = h()
    println!(g(), g(), x.name, y.name)
    println!(s, box.name)
    return 0
}
"#;

#[test]
fn captured_buffer_return_survives_repeated_calls() {
    for backend in ["interpreter", "cranelift", "llvm"] {
        let dir = scratch::dir(&format!("capture-buffer-{backend}"));
        let out = run(&dir, BUFFER_RETURN, backend);
        assert_eq!(out, "5 5 5 5\n5", "{backend}");
    }
}

#[test]
fn captured_string_and_object_returns_survive_repeated_calls() {
    for backend in ["interpreter", "cranelift", "llvm"] {
        let dir = scratch::dir(&format!("capture-object-{backend}"));
        let out = run(&dir, STRING_AND_OBJECT_RETURN, backend);
        assert_eq!(out, "captured captured boxed boxed\ncaptured boxed", "{backend}");
    }
}
