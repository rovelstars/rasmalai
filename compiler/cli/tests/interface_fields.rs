#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::path::Path;

// An interface-typed field holds a concrete class chosen at runtime. Field
// teardown therefore cannot use a static instance size: it must release
// through the heap registry, which already records the size and dtor of each
// allocated object. These cases pin that path on both codegen backends, the
// interpreter, and optimized artifacts.

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn run_backend(dir: &Path, backend: &str, jobs: usize) -> String {
    let out = std::process::Command::new(rnx())
        .arg("run")
        .arg("--backend")
        .arg(backend)
        .arg("--jobs")
        .arg(jobs.to_string())
        .arg(dir.join("main.rnx"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{backend} jobs={jobs}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn artifact_of(stdout: &str) -> Option<std::path::PathBuf> {
    stdout.lines().rev().find_map(|l| {
        l.strip_prefix("artifact: ")
            .map(|s| std::path::PathBuf::from(s.trim().trim_end_matches(" (fresh)")))
    })
}

fn check(src: &str, want: &str, tag: &str) {
    let dir = crate::scratch::dir(&format!("ifacefield-{tag}"));
    std::fs::write(dir.join("main.rnx"), src).unwrap();
    for backend in ["interpreter", "cranelift", "llvm"] {
        for jobs in [1, 4] {
            assert_eq!(run_backend(dir.path(), backend, jobs), want, "{backend} jobs={jobs} {tag}");
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
        assert_eq!(run.status.code(), Some(0), "aot debug={debug} {tag} exit");
        assert_eq!(String::from_utf8_lossy(&run.stdout), want, "aot debug={debug} {tag}");
    }
}

const SCENE: &str = "interface Shape { fn area(): Int; }\n\
class Square : Shape { let s: Int; init(s: Int) { this.s = s; } fn area(): Int { return this.s * this.s; } }\n\
class Rect : Shape { let w: Int; let h: Int; init(w: Int, h: Int) { this.w = w; this.h = h; } fn area(): Int { return this.w * this.h; } }\n\
class Scene {\n\
    let primary: Shape;\n\
    let shapes: Array<Shape>;\n\
    init(s: Shape) { this.primary = s; this.shapes = []; }\n\
    fn add(s: Shape) { this.shapes.push(s); }\n\
    fn total(): Int {\n\
        let sum = this.primary.area();\n\
        let i = 0;\n\
        while i < this.shapes.len() { sum = sum + this.shapes[i].area(); i = i + 1; }\n\
        return sum;\n\
    }\n\
}\n\
fn Main(): Int {\n\
    let sc = new Scene(new Square(3));\n\
    sc.add(new Rect(2, 5));\n\
    sc.add(new Square(4));\n\
    println!(sc.total());\n\
    sc.primary = new Rect(1, 7);\n\
    println!(sc.total());\n\
    return 0;\n\
}\n";

#[test]
fn interface_field_holds_two_concrete_classes() {
    check(SCENE, "35\n33\n", "scene");
}

#[test]
fn interface_field_reassigned_on_teardown() {
    check(
        "interface Named { fn label(): String; }\n\
class A : Named { fn label(): String { return \"a\"; } }\n\
class B : Named { fn label(): String { return \"b\"; } }\n\
class Slot { let cur: Named; init(n: Named) { this.cur = n; } fn set(n: Named) { this.cur = n; } }\n\
fn Main(): Int {\n\
    let s = new Slot(new A());\n\
    println!(s.cur.label());\n\
    s.set(new B());\n\
    println!(s.cur.label());\n\
    s.set(new A());\n\
    println!(s.cur.label());\n\
    return 0;\n\
}\n",
        "a\nb\na\n",
        "reassign",
    );
}

#[test]
fn interface_field_nullable_optional() {
    check(
        "interface Named { fn label(): String; }\n\
class A : Named { fn label(): String { return \"a\"; } }\n\
class Slot { let cur: Named?; init() { this.cur = null; } fn set(n: Named?) { this.cur = n; } }\n\
fn Main(): Int {\n\
    let s = new Slot();\n\
    println!(s.cur?.label() ?? \"none\");\n\
    s.set(new A());\n\
    println!(s.cur?.label() ?? \"none\");\n\
    s.set(null);\n\
    println!(s.cur?.label() ?? \"none\");\n\
    return 0;\n\
}\n",
        "none\na\nnone\n",
        "optional",
    );
}
