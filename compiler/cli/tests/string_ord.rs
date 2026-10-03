use std::path::PathBuf;

const ORD_SRC: &str = "import { JSON } from \"@std/json\";\nfn Main(): Int {\n    if !(\"abc\" < \"abd\") { print(\"FAIL1\"); return 1; }\n    if !(\"abd\" > \"abc\") { print(\"FAIL2\"); return 1; }\n    if !(\"foo\" < \"foobar\") { print(\"FAIL3\"); return 1; }\n    if !(\"foobar\" > \"foo\") { print(\"FAIL4\"); return 1; }\n    if !(\"same\" <= \"same\") { print(\"FAIL5\"); return 1; }\n    if !(\"same\" >= \"same\") { print(\"FAIL6\"); return 1; }\n    if \"b\" <= \"a\" { print(\"FAIL7\"); return 1; }\n    if \"a\" >= \"b\" { print(\"FAIL8\"); return 1; }\n    if !(\"\" < \"a\") { print(\"FAIL9\"); return 1; }\n    if !(\"Z\" < \"a\") { print(\"FAIL10\"); return 1; }\n    if !(\"10\" < \"9\") { print(\"FAIL11\"); return 1; }\n    if !(\"compress\" < \"deflate\") { print(\"FAIL12\"); return 1; }\n    let m = JSON.parseObject(\"\\{\\\"b\\\": \\\"compressBound\\\", \\\"a\\\": \\\"compress2\\\"}\");\n    let x = \"\" + m.get(\"b\");\n    let y = \"\" + m.get(\"a\");\n    if !(y < x) { print(\"FAIL13\"); return 1; }\n    if !(x > y) { print(\"FAIL14\"); return 1; }\n    if !(y <= y) { print(\"FAIL15\"); return 1; }\n    print(\"ord-ok\");\n    return 0;\n}\n";

fn write_proj(src: &str, tag: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-strord-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Project.config"),
        "export default {\n    project: {\n        name: \"t\",\n        version: \"0.1.0\"\n    }\n}\n",
    )
    .unwrap();
    let main = dir.join("src").join("main.rnx");
    std::fs::write(&main, src).unwrap();
    (dir, main)
}

fn rnx() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rnx"))
}

#[test]
fn string_ordering_is_lexicographic_on_all_backends() {
    let (dir, main) = write_proj(ORD_SRC, "ord");
    for backend in ["interpreter", "cranelift", "llvm"] {
        let out = std::process::Command::new(rnx())
            .arg("run")
            .arg("--backend")
            .arg(backend)
            .arg(&main)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{backend}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            String::from_utf8(out.stdout).unwrap(),
            "ord-ok\n",
            "{backend}"
        );
    }
    let bin = dir.join("t_ord");
    let build = std::process::Command::new(rnx())
        .arg("build")
        .arg(&main)
        .arg("-o")
        .arg(&bin)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new(&bin).output().unwrap();
    assert!(run.status.success());
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "ord-ok\n");
    let _ = std::fs::remove_dir_all(&dir);
}
