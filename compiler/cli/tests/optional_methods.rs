#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::path::Path;

// `?.field` and `?.method()` both parse to an optional access with zero
// arguments, so the lowering must key off the node kind. Reading the argument
// count made every zero-argument optional method call lower as a field read,
// which failed at runtime with `unknown field`. These cases cover the call
// shape, argument passing, chains, and interface receivers.

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
    assert!(out.status.success(), "{backend}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

fn artifact_of(stdout: &str) -> Option<std::path::PathBuf> {
    stdout.lines().rev().find_map(|l| {
        l.strip_prefix("artifact: ")
            .map(|s| std::path::PathBuf::from(s.trim().trim_end_matches(" (fresh)")))
    })
}

fn check(src: &str, want: &str, tag: &str) {
    let dir = crate::scratch::dir(&format!("optmethod-{tag}"));
    std::fs::write(dir.join("main.rnx"), src).unwrap();
    for backend in ["interpreter", "cranelift", "llvm"] {
        assert_eq!(run_backend(dir.path(), backend), want, "{backend} {tag}");
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
        assert_eq!(run.status.code(), Some(0), "aot debug={debug} {tag} exit");
        assert_eq!(String::from_utf8_lossy(&run.stdout), want, "aot debug={debug} {tag}");
    }
}

#[test]
fn optional_method_without_args() {
    check(
        "class N { fn label(): String { return \"n\"; } }\n\
class S { let cur: N?; init() { this.cur = null; } fn set(n: N?) { this.cur = n; } }\n\
fn Main(): Int {\n\
    let s = new S();\n\
    println!(s.cur?.label() ?? \"none\");\n\
    s.set(new N());\n\
    println!(s.cur?.label() ?? \"none\");\n\
    return 0;\n\
}\n",
        "none\nn\n",
        "noargs",
    );
}

#[test]
fn optional_method_with_args() {
    check(
        "class Acc { let n: Int; init(n: Int) { this.n = n; } fn plus(x: Int): Int { return this.n + x; } }\n\
class S { let acc: Acc?; init() { this.acc = null; } fn set(a: Acc?) { this.acc = a; } }\n\
fn Main(): Int {\n\
    let s = new S();\n\
    println!(s.acc?.plus(2) ?? -1);\n\
    s.set(new Acc(40));\n\
    println!(s.acc?.plus(2) ?? -1);\n\
    return 0;\n\
}\n",
        "-1\n42\n",
        "args",
    );
}

#[test]
fn optional_chain_field_then_method() {
    check(
        "class Inner { fn tag(): String { return \"inner\"; } }\n\
class Mid { let inner: Inner?; init(i: Inner?) { this.inner = i; } }\n\
class Root { let mid: Mid?; init(m: Mid?) { this.mid = m; } }\n\
fn Main(): Int {\n\
    let a = new Root(null);\n\
    let b = new Root(new Mid(null));\n\
    let c = new Root(new Mid(new Inner()));\n\
    println!(a.mid?.inner?.tag() ?? \"none\");\n\
    println!(b.mid?.inner?.tag() ?? \"none\");\n\
    println!(c.mid?.inner?.tag() ?? \"none\");\n\
    return 0;\n\
}\n",
        "none\nnone\ninner\n",
        "chain",
    );
}

#[test]
fn optional_method_on_interface_field() {
    check(
        "interface Named { fn label(): String; fn score(): Int; }\n\
class A : Named { fn label(): String { return \"a\"; } fn score(): Int { return 1; } }\n\
class Slot { let cur: Named?; init() { this.cur = null; } fn set(n: Named?) { this.cur = n; } }\n\
fn Main(): Int {\n\
    let s = new Slot();\n\
    println!(s.cur?.label() ?? \"none\");\n\
    println!(s.cur?.score() ?? 0);\n\
    s.set(new A());\n\
    println!(s.cur?.label() ?? \"none\");\n\
    println!(s.cur?.score() ?? 0);\n\
    return 0;\n\
}\n",
        "none\n0\na\n1\n",
        "iface",
    );
}
