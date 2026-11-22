#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::path::Path;

// Pins for the ownership defects that only showed up once release became the
// default build. Each case ran clean on the interpreter while a native backend
// aborted with `rnx_release on zero count` or `rnx_free size mismatch`.

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn run_backend(dir: &Path, backend: &str) -> String {
    let out = std::process::Command::new(rnx())
        .arg("run")
        .arg("--backend")
        .arg(backend)
        .arg(dir.join("main.rnx"))
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !text.contains("rnx_release on zero count") && !text.contains("rnx_free size mismatch"),
        "{backend} aborted:\n{text}"
    );
    assert!(out.status.success(), "{backend}: {text}");
    String::from_utf8(out.stdout).unwrap()
}

fn artifact_of(stdout: &str) -> Option<std::path::PathBuf> {
    stdout.lines().rev().find_map(|l| {
        l.strip_prefix("artifact: ")
            .map(|s| std::path::PathBuf::from(s.trim().trim_end_matches(" (fresh)")))
    })
}

fn check(src: &str, want: Option<&str>, tag: &str) {
    let dir = crate::scratch::dir(&format!("defect-{tag}"));
    std::fs::write(dir.join("main.rnx"), src).unwrap();
    for backend in ["interpreter", "cranelift", "llvm"] {
        let got = run_backend(dir.path(), backend);
        if let Some(want) = want {
            assert_eq!(got, want, "{backend} {tag}");
        }
    }
    for debug in [false, true] {
        let mut build = std::process::Command::new(rnx());
        build.arg("build").arg(dir.join("main.rnx"));
        if debug {
            build.arg("--debug");
        }
        let out = build.output().unwrap();
        assert!(
            out.status.success(),
            "build debug={debug}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let bin = artifact_of(&String::from_utf8_lossy(&out.stdout)).expect("artifact path");
        let run = std::process::Command::new(bin).output().unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&run.stdout),
            String::from_utf8_lossy(&run.stderr)
        );
        assert!(!text.contains("rnx_release on zero count"), "aot debug={debug}:\n{text}");
        assert!(!text.contains("rnx_free size mismatch"), "aot debug={debug}:\n{text}");
        assert_eq!(run.status.code(), Some(0), "aot debug={debug} {tag}:\n{text}");
        if let Some(want) = want {
            assert_eq!(String::from_utf8_lossy(&run.stdout), want, "aot debug={debug} {tag}");
        }
    }
}

#[test]
fn optional_class_field_teardown() {
    check(
        "class Node {\n\
    let value: Int;\n\
    let next: Node?;\n\
    init(v: Int, n: Node?) { this.value = v; this.next = n; }\n\
    deinit { println!(\"drop\", this.value); }\n\
}\n\
fn Main(): Int {\n\
    let head: Node? = null;\n\
    let i = 0;\n\
    while i < 5000 {\n\
        head = new Node(i, head);\n\
        i = i + 1;\n\
    }\n\
    let count = 0;\n\
    let cur = head;\n\
    while cur != null { count = count + 1; cur = cur.next; }\n\
    println!(count);\n\
    head = null;\n\
    cur = null;\n\
    println!(\"clean\");\n\
    return 0;\n\
}\n",
        None,
        "optional",
    );
}

#[test]
fn any_cast_across_call_boundary() {
    check(
        "class Leaf { let w: Int; init(w: Int) { this.w = w; } fn weight(): Int { return this.w; } }\n\
fn take(a: Any): Int { let lf = a as Leaf; return lf.weight(); }\n\
fn Main(): Int {\n\
    let held = new Leaf(7);\n\
    println!(take(held));\n\
    println!(take(new Leaf(9)));\n\
    let slot: Any = new Leaf(11);\n\
    let back = slot as Leaf;\n\
    println!(back.weight());\n\
    return 0;\n\
}\n",
        Some("7\n9\n11\n"),
        "anycast",
    );
}

#[test]
fn any_cast_into_interface_field() {
    check(
        "interface Weighted { fn weight(): Int; }\n\
class Leaf : Weighted { let w: Int; init(w: Int) { this.w = w; } fn weight(): Int { return this.w; } }\n\
class Holder { let item: Weighted; init(v: Any) { this.item = v as Weighted; } }\n\
fn Main(): Int {\n\
    let h = new Holder(new Leaf(13));\n\
    println!(h.item.weight());\n\
    return 0;\n\
}\n",
        Some("13\n"),
        "castiface",
    );
}
