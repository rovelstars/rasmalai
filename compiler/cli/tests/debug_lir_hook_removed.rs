#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::process::Command;

#[test]
fn debug_lir_hook_is_gone() {
    let dir = crate::scratch::dir("debug-lir-hook");
    std::fs::write(
        dir.join("main.rnx"),
        "fn gradeMult(a: Int, b: Int): Int {\n    return a * b;\n}\nfn Main(): Int {\n    println!(gradeMult(6, 7))\n    return 0\n}\n",
    )
    .unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_rnx"))
        .arg("run")
        .arg("--backend")
        .arg("interpreter")
        .arg(dir.join("main.rnx"))
        .env("RNX_DEBUG_LIR", "1")
        .env("NO_COLOR", "1")
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "42\n");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("dbg lir"),
        "RNX_DEBUG_LIR must not dump LIR for a hardcoded fixture name: {stderr}"
    );
}
