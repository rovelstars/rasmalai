use std::path::PathBuf;
use std::process::Command;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-llvmasync-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run_jit(dir: &PathBuf, src: &str) -> std::process::Output {
    let prog = dir.join("main.rnx");
    std::fs::write(&prog, src).unwrap();
    Command::new(rnx())
        .arg("run")
        .env("NO_COLOR", "1")
        .arg("--backend")
        .arg("llvm")
        .arg(&prog)
        .output()
        .unwrap()
}

fn expect_ok(dir_label: &str, src: &str, want: &str) {
    let dir = fresh_dir(dir_label);
    let out = run_jit(&dir, src);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "jit failed: {stderr}");
    assert_eq!(stdout, want, "stdout mismatch");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn async_array_return_and_use() {
    expect_ok(
        "arr",
        "async fn maker(): Promise<Array<Int>> {\n    let out: Array<Int> = [1, 2];\n    return out;\n}\nlet a: Array<Int> = await maker();\nprint(a.length());\n",
        "2\n",
    );
}

#[test]
fn async_object_return_and_member() {
    expect_ok(
        "obj",
        "class Box {\n    let v: Int = 0;\n    init(v: Int) { this.v = v; }\n}\nasync fn maker(): Promise<Box> {\n    let out: Box = new Box(9);\n    return out;\n}\nlet a: Box = await maker();\nprint(a.v);\n",
        "9\n",
    );
}

#[test]
fn async_capture_loop_data_integrity() {
    expect_ok(
        "caploop",
        "async fn fetchData(n: Int): Promise<Array<Int>> {\n    let out: Array<Int> = [];\n    let i = 0;\n    while i < n {\n        out.push(i * 3 + 1);\n        i = i + 1;\n    }\n    return out;\n}\nasync fn useData(tag: String, seed: Int): Promise<Int> {\n    let data: Array<Int> = await fetchData(seed);\n    let s = 0;\n    let i = 0;\n    while i < data.length() {\n        s = s + data[i];\n        i = i + 1;\n    }\n    print(tag);\n    return s;\n}\nlet a: Int = await useData(\"got\", 4);\nprint(a);\n",
        "got\n22\n",
    );
}

#[test]
fn async_erased_generic_container() {
    expect_ok(
        "gen",
        "fn Main(): Int {\n    let tmp: Array<Int> = [1, 2];\n    let o: Array<Int>? = tmp;\n    let e: Array<Int> = [];\n    let a: Array<Int> = o ?? e;\n    print(a.length());\n    let n: Array<Int>? = null;\n    let b: Array<Int> = n ?? e;\n    print(b.length());\n    return 0;\n}\n",
        "2\n0\n",
    );
}

#[test]
fn async_string_capture() {
    expect_ok(
        "str",
        "async fn maker(): Promise<String> {\n    let out: String = \"hello\";\n    return out;\n}\nlet a: String = await maker();\nprint(a.length());\n",
        "5\n",
    );
}
