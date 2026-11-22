#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::path::Path;

// A closure passed to a generic function is invoked from the erased callee,
// which sees `fn(Any): Any` and cannot know the closure's concrete return
// type. The closure ABI therefore boxes a scalar result whenever the generic
// parameter erases its return. The sequential lowering path and the parallel
// path each build the closure shell independently, so both job counts run:
// the original crash only appeared with jobs > 1.

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
    let dir = crate::scratch::dir(&format!("genclo-{tag}"));
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

#[test]
fn generic_closure_scalar_result() {
    check(
        "fn apply<T>(v: T, f: fn(T): T): T { return f(v); }\nfn Main(): Int {\n    println!(apply(41, (x: Int): Int => x + 1));\n    return 0;\n}\n",
        "42\n",
        "int",
    );
}

#[test]
fn generic_closure_bool_result() {
    check(
        "fn apply<T>(v: T, f: fn(T): Bool): Bool { return f(v); }\nfn Main(): Int {\n    println!(apply(3, (x: Int): Bool => x > 2));\n    return 0;\n}\n",
        "true\n",
        "bool",
    );
}

#[test]
fn generic_closure_string_result() {
    check(
        "fn apply<T>(v: T, f: fn(T): String): String { return f(v); }\nfn Main(): Int {\n    println!(apply(7, (x: Int): String => \"n=\" + x.toString()));\n    return 0;\n}\n",
        "n=7\n",
        "str",
    );
}

#[test]
fn generic_closure_captures_outer_value() {
    check(
        "fn apply<T>(v: T, f: fn(T): T): T { return f(v); }\nfn Main(): Int {\n    let bump = 10;\n    println!(apply(5, (x: Int): Int => x + bump));\n    return 0;\n}\n",
        "15\n",
        "capture",
    );
}

#[test]
fn generic_hof_maps_an_array() {
    check(
        "fn mapAll<T>(xs: Array<T>, f: fn(T): T): Array<T> {\n    let out: Array<T> = [];\n    let i = 0;\n    while i < xs.len() { out.push(f(xs[i])); i = i + 1; }\n    return out;\n}\nfn Main(): Int {\n    let got = mapAll([1, 2, 3], (x: Int): Int => x * 3);\n    println!(got.join(\",\"));\n    return 0;\n}\n",
        "3,6,9\n",
        "map",
    );
}

#[test]
fn generic_closure_used_as_value() {
    check(
        "fn twice<T>(v: T, f: fn(T): T): T { return f(f(v)); }\nfn Main(): Int {\n    let n = twice(2, (x: Int): Int => x + 3);\n    let s = twice(\"ab\", (x: String): String => x + \"!\");\n    println!(n, s);\n    return 0;\n}\n",
        "8 ab!!\n",
        "twice",
    );
}
