#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::process::Command;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn run(src: &str, backend: &str) -> String {
    let dir = crate::scratch::dir("compound-string");
    std::fs::write(
        dir.join("Project.config"),
        "export default {\n    project: { name: \"cs\", version: \"0.1.0\" },\n    entries: { main: \"main.rnx\" },\n}\n",
    )
    .unwrap();
    std::fs::write(dir.join("main.rnx"), src).unwrap();
    let out = Command::new(rnx())
        .args(["run", "--backend", backend])
        .current_dir(&dir)
        .env("NO_COLOR", "1")
        .env(
            "RNX_VENDOR_DIR",
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../stdlib/src"),
        )
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "{backend}: {stderr}{stdout}");
    assert!(
        !stderr.contains("panicked") && !stderr.contains("rnx panic"),
        "{backend}: {stderr}"
    );
    stdout.trim().to_string()
}

fn check_all(src: &str, want: &str) {
    for backend in ["interpreter", "cranelift", "llvm"] {
        assert_eq!(run(src, backend), want, "{backend}");
    }
}

#[test]
fn plus_assign_concatenates_a_string_binding() {
    check_all(
        "fn Main(): Int {\n    let s = \"\"\n    s += \"a\"\n    s += \"b\"\n    let n = 1\n    n += 2\n    println!(s, n)\n    return 0\n}\n",
        "ab 3",
    );
}

#[test]
fn plus_assign_concatenates_a_string_field() {
    check_all(
        "class P { let s: String = \"\" }\nfn Main(): Int {\n    let p = new P()\n    p.s += \"a\"\n    p.s += \"b\"\n    println!(p.s)\n    return 0\n}\n",
        "ab",
    );
}

#[test]
fn plus_assign_concatenates_a_string_element() {
    check_all(
        "fn Main(): Int {\n    let a: Array<String> = []\n    a.push(\"x\")\n    a[0] += \"y\"\n    a[0] += \"z\"\n    println!(a[0])\n    return 0\n}\n",
        "xyz",
    );
}
