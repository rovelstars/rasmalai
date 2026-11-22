#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::path::{Path, PathBuf};

// The in-house native linker folds identical code, garbage-collects sections,
// and strips symbols. The system linker does none of that. A miscompile in the
// in-house path can therefore survive every test that only checks the artifact
// exists, so build each program both ways and require identical output.
//
// This gate exists because ICF once hashed relocation targets by section
// content: two zero-initialized statics shared bytes, their readers folded
// together, and `println!([1, 2, 3])` dereferenced an integer as a pointer.

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn system_linker_available() -> bool {
    ["cc", "gcc", "clang"]
        .iter()
        .any(|c| std::process::Command::new(c).arg("--version").output().is_ok())
}

fn artifact_of(stdout: &str) -> Option<PathBuf> {
    stdout.lines().rev().find_map(|l| {
        l.strip_prefix("artifact: ")
            .map(|s| PathBuf::from(s.trim().trim_end_matches(" (fresh)")))
    })
}

fn build_and_run(root: &Path, native: bool) -> (String, i32) {
    let main = root.join("main.rnx");
    let build_dir = root.join(".rnx-cache").join("build");
    let _ = std::fs::remove_dir_all(&build_dir);
    let out = std::process::Command::new(rnx())
        .arg("build")
        .arg(&main)
        .env("RNX_NATIVE_LINK", if native { "1" } else { "0" })
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success(),
        "build with native={native} failed:\n{text}"
    );
    let bin = artifact_of(&String::from_utf8_lossy(&out.stdout))
        .unwrap_or_else(|| panic!("build printed no artifact:\n{text}"));
    let run = std::process::Command::new(&bin).output().unwrap();
    (
        String::from_utf8_lossy(&run.stdout).into_owned(),
        run.status.code().unwrap_or(-1),
    )
}

fn differential(tag: &str, src: &str) {
    if !cfg!(target_os = "linux") {
        return;
    }
    if !system_linker_available() {
        eprintln!("skip {tag}: no system C compiler for the differential leg");
        return;
    }
    let root = crate::scratch::dir(&format!("linkdiff-{tag}"));
    std::fs::write(root.join("main.rnx"), src).unwrap();
    let native = build_and_run(root.path(), true);
    let system = build_and_run(root.path(), false);
    assert_eq!(
        native, system,
        "{tag}: in-house linker output differs from the system linker"
    );
}

const CASES: &[(&str, &str)] = &[
    (
        "array-print",
        "fn Main(): Int {\n    println!([1, 2, 3]);\n    println!([[1, 2], [3]]);\n    return 0;\n}\n",
    ),
    (
        "array-of-any",
        "fn Main(): Int {\n    let a: Array<Any> = [];\n    a.push(1);\n    a.push(\"two\");\n    a.push(true);\n    println!(a);\n    return 0;\n}\n",
    ),
    (
        "map-set",
        "import { Map, Set } from \"@std/collections\";\nfn Main(): Int {\n    let m = new Map<String, Int>();\n    m.set(\"b\", 2);\n    m.set(\"a\", 1);\n    let s = new Set<String>();\n    s.add(\"x\");\n    println!(m, s);\n    return 0;\n}\n",
    ),
    (
        "class-optional",
        "class Node {\n    let name: String;\n    let child: Node?;\n    init(name: String, child: Node?) { this.name = name; this.child = child; }\n}\nfn Main(): Int {\n    let root = new Node(\"root\", new Node(\"leaf\", null));\n    println!(root?.child?.name ?? \"none\");\n    return 0;\n}\n",
    ),
    (
        "closure-hof",
        "fn Main(): Int {\n    let xs = [1, 2, 3, 4];\n    let out = xs.map((n) => n * 2).filter((n) => n > 4);\n    println!(out.join(\",\"));\n    return 0;\n}\n",
    ),
    (
        "generic-closure",
        "fn apply<T>(v: T, f: fn(T): T): T { return f(v); }\nfn Main(): Int {\n    println!(apply(41, (x: Int): Int => x + 1));\n    return 0;\n}\n",
    ),
    (
        "interface-field",
        "interface Shape { fn area(): Int; }\nclass Square : Shape { let s: Int; init(s: Int) { this.s = s; } fn area(): Int { return this.s * this.s; } }\nclass Holder { let shape: Shape; init(shape: Shape) { this.shape = shape; } }\nfn Main(): Int {\n    let h = new Holder(new Square(3));\n    println!(h.shape.area());\n    return 0;\n}\n",
    ),
    (
        "json",
        "import { JSON } from \"@std/json\";\nfn Main(): Int {\n    let v = JSON.parse(\"[1, 2, 3]\");\n    println!(JSON.stringify(v));\n    return 0;\n}\n",
    ),
    (
        "floats",
        "fn Main(): Int {\n    let x = 1.5;\n    let y = x * 2.0 + 0.25;\n    println!(y);\n    return 0;\n}\n",
    ),
    (
        "enum-switch",
        "enum Color { Red, Green, Blue }\nfn name(c: Color): String {\n    switch c {\n        case .Red: return \"red\";\n        case .Green: return \"green\";\n        case .Blue: return \"blue\";\n    }\n}\nfn Main(): Int {\n    println!(name(Color.Red), name(Color.Green), name(Color.Blue));\n    return 0;\n}\n",
    ),
    (
        "strings-loop",
        "fn Main(): Int {\n    let out = \"\";\n    let i = 0;\n    while i < 20 {\n        out = out + i.toString();\n        i = i + 1;\n    }\n    println!(out.len(), out.slice(0, 6));\n    return 0;\n}\n",
    ),
];

#[test]
fn in_house_linker_matches_system_linker() {
    for (tag, src) in CASES {
        differential(tag, src);
    }
}
