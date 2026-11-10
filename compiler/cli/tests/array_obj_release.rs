#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::path::PathBuf;
use std::process::Command;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .expect("repo root")
}

const SRC: &str = r#"import { liveCount } from "@std/testing";
class Box {
    let v: Int;
    init(v: Int) { this.v = v; }
}
fn useArray(): Int {
    let b = new Box(9);
    let arr: Array<Box> = [b];
    arr = [];
    return b.v;
}
fn Main(): Int {
    let before = liveCount();
    let v = useArray();
    let after = liveCount();
    println!("live delta:", after - before, "v:", v);
    return 0;
}
"#;

#[test]
fn object_array_elements_release_through_arc() {
    let dir = crate::scratch::dir("array-obj-release");
    std::fs::write(dir.join("main.rnx"), SRC).unwrap();
    for backend in ["interpreter", "cranelift", "llvm"] {
        let out = Command::new(rnx())
            .arg("run")
            .arg("--backend")
            .arg(backend)
            .arg(dir.join("main.rnx"))
            .current_dir(&dir)
            .env("NO_COLOR", "1")
            .env("RNX_VENDOR_DIR", repo().join("compiler/stdlib/src"))
            .output()
            .unwrap();
        let stdout = String::from_utf8(out.stdout).unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{backend} failed: {stderr}");
        assert_eq!(
            stdout, "live delta: 0 v: 9\n",
            "{backend} leaked or double-freed array elements"
        );
    }
}
