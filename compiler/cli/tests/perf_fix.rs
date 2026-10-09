use std::path::Path;
use std::process::Command;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn proj(tag: &str, src: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("rnx-perffix-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("main.rnx"), src).unwrap();
    root
}

fn build(root: &std::path::Path, args: &[&str]) -> (i32, String, String) {
    let mut cmd = Command::new(rnx());
    cmd.arg("build")
        .arg("--no-color")
        .arg(root.join("main.rnx"));
    for a in args {
        cmd.arg(a);
    }
    cmd.env_remove("RNX_DENY_PERF_LINTS");
    cmd.env_remove("RNX_SHOW_PERF_NOTES");
    let out = cmd.output().unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

fn sha256(path: &Path) -> String {
    let out = Command::new("sha256sum").arg(path).output().unwrap();
    assert!(out.status.success(), "sha256sum failed");
    String::from_utf8(out.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string()
}

fn artifact(stdout: &str) -> String {
    stdout
        .lines()
        .rev()
        .find_map(|l| l.strip_prefix("artifact: "))
        .map(|s| s.trim().trim_end_matches(" (fresh)").to_string())
        .expect("build prints artifact path")
}

fn run_bin(path: &str) -> (i32, Vec<u8>) {
    let out = Command::new(path).output().unwrap();
    (out.status.code().unwrap(), out.stdout)
}

const BASE: &str = "fn Main(): Int {\n    let n = 16;\n    let a: Array<Int> = [];\n    let b: Array<Int> = [];\n    let c: Array<Int> = [];\n    let i = 0;\n    while i < n * n {\n        a.push(0);\n        b.push(0);\n        @@C@@\n        i = i + 1;\n    }\n    let r = 0;\n    while r < n {\n        let k = 0;\n        while k < n {\n            a[r * n + k] = (r + k) % 64;\n            b[r * n + k] = (r * k) % 64;\n            k = k + 1;\n        }\n        r = r + 1;\n    }\n    @@EXTRA@@let x = 0;\n    while x < n {\n        let y = 0;\n        while y < n {\n            let s = 0;\n            let z = 0;\n            while z < n {\n                s = s + a[x * n + z] * b[z * n + y];\n                z = z + 1;\n            }\n            @@STORE@@\n            y = y + 1;\n        }\n        x = x + 1;\n    }\n    let total = 0;\n    let t = 0;\n    while t < n * n {\n        total = total + c[t];\n        t = t + 1;\n    }\n    print(\"RESULT checksum\", total);\n    return 0;\n}\n";

fn garbage() -> String {
    BASE.replace("@@C@@", "c.push(7);")
        .replace("@@EXTRA@@", "")
        .replace("@@STORE@@", "c[x * n + y] = s;")
}

fn expected_fixed() -> String {
    BASE.replace("@@C@@", "c.push(0);")
        .replace("@@EXTRA@@", "")
        .replace("@@STORE@@", "c[x * n + y] = s;")
}

fn aliased() -> String {
    BASE.replace("@@C@@", "c.push(0);")
        .replace("@@EXTRA@@", "")
        .replace("@@STORE@@", "a[x * n + y] = s;")
}

fn costgate() -> String {
    expected_fixed().replacen("while y < n {", "while y < 4 {", 1)
}

fn subscript() -> String {
    expected_fixed().replacen("a[x * n + z]", "a[y * n + z]", 1)
}

fn rollback_case() -> String {
    BASE.replace("@@C@@", "c.push(7);")
        .replace(
            "@@EXTRA@@",
            "    let j = 0;\n    while j < n {\n        c.push(0);\n        j = j + 1;\n    }\n",
        )
        .replace("@@STORE@@", "c[x * n + y] = s;")
}

const SEARCH: &str = "fn Main(): Int {\n    let n = 64;\n    let m = 32;\n    let keys: Array<Int> = [];\n    let table: Array<Int> = [];\n    let out: Array<Int> = [];\n    let i = 0;\n    while i < 2048 {\n        keys.push(i % 97);\n        table.push((i * 3) % 101);\n        out.push(0);\n        i = i + 1;\n    }\n    let o = 0;\n    while o < n {\n        let needle = keys[o];\n        let found = -1;\n        let k = 0;\n        while k < m {\n            if table[k] == needle {\n                found = k;\n                break;\n            }\n            k = k + 1;\n        }\n        out[o] = found;\n        o = o + 1;\n    }\n    return out[0] + out[1];\n}\n";

const CONCAT: &str = "fn Main(): Int {\n    let s = \"\";\n    let i = 0;\n    while i < 64 {\n        s = s + \"x\";\n        i = i + 1;\n    }\n    print(\"built\", s);\n    return 0;\n}\n";

const HELLO: &str = "fn Main(): Int {\n    print(\"hello\");\n    return 0;\n}\n";

#[test]
fn fix_converts_garbage_fill_to_zero_fill() {
    let root = proj("e2e", &garbage());
    let main = root.join("main.rnx");
    let pre_sha = sha256(&main);
    let (code, stdout, stderr) = build(&root, &[]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stderr.contains("warning[P201]"), "{stderr}");
    let pre_bin = artifact(&stdout);
    let (pre_exit, pre_out) = run_bin(&pre_bin);
    assert_eq!(pre_exit, 0);
    assert!(pre_out.windows(8).any(|w| w == b"checksum"), "{pre_out:?}");

    let (code, stdout, stderr) = build(&root, &["--fix"]);
    assert_eq!(code, 0, "{stdout}\n{stderr}");
    assert!(stdout.contains("-        c.push(7);"), "{stdout}");
    assert!(stdout.contains("+        c.push(0);"), "{stdout}");
    assert!(stdout.contains("fixed:"), "{stdout}");

    let fixed = std::fs::read_to_string(&main).unwrap();
    assert_eq!(fixed, expected_fixed());
    assert!(!main.with_extension("rnx.bak").exists() && !root.join("main.rnx.bak").exists());
    assert_ne!(sha256(&main), pre_sha);

    let (_, _, stderr) = build(&root, &[]);
    assert!(!stderr.contains("warning[P201]"), "{stderr}");
    let (code, stdout, stderr) = build(&root, &["--show-perf-notes"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stderr.contains("note[P101]"), "{stderr}");
    let post_bin = artifact(&stdout);
    let (post_exit, post_out) = run_bin(&post_bin);
    assert_eq!((pre_exit, &pre_out), (post_exit, &post_out));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn fix_keep_backup_preserves_bak() {
    let root = proj("keepbak", &garbage());
    let main = root.join("main.rnx");
    let bak = root.join("main.rnx.bak");
    let (code, _, stderr) = build(&root, &["--fix", "--fix-keep-backup"]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(std::fs::read_to_string(&main).unwrap(), expected_fixed());
    assert!(bak.exists(), "backup must be kept");
    assert!(
        std::fs::read_to_string(&bak)
            .unwrap()
            .contains("c.push(7);")
    );
    let _ = std::fs::remove_dir_all(&root);
}

fn assert_refused(tag: &str, src: &str, needle: &str) {
    let root = proj(tag, src);
    let main = root.join("main.rnx");
    let pre = std::fs::read(&main).unwrap();
    let (code, _, stderr) = build(&root, &["--fix"]);
    assert_eq!(code, 1, "{stderr}");
    assert!(stderr.contains("no autofix available"), "{stderr}");
    assert!(stderr.contains(needle), "{stderr}");
    assert_eq!(std::fs::read(&main).unwrap(), pre);
    assert!(!root.join("main.rnx.bak").exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn fix_refuses_alias_failure() {
    assert_refused("alias", &aliased(), "P201");
}

#[test]
fn fix_refuses_costgate_failure() {
    let src = costgate();
    let root = proj("costgate-probe", &src);
    let (_, _, stderr) = build(&root, &[]);
    assert!(stderr.contains("warning[P201]"), "{stderr}");
    let _ = std::fs::remove_dir_all(&root);
    assert_refused("costgate", &src, "P201");
}

#[test]
fn fix_refuses_subscript_failure() {
    assert_refused("subscript", &subscript(), "P201");
}

#[test]
fn fix_refuses_search_shape() {
    let root = proj("search-probe", SEARCH);
    let (_, _, stderr) = build(&root, &[]);
    assert!(stderr.contains("warning[P202]"), "{stderr}");
    let _ = std::fs::remove_dir_all(&root);
    assert_refused("search", SEARCH, "no autofix available for P202");
}

#[test]
fn fix_leaves_concat_shape_untouched_without_lint() {
    let root = proj("concat-probe", CONCAT);
    let main = root.join("main.rnx");
    let pre = std::fs::read(&main).unwrap();
    let (code, stdout, stderr) = build(&root, &[]);
    assert_eq!(code, 0, "{stderr}");
    assert!(!stderr.contains("warning[P2"), "{stderr}");
    let (code, stdout, stderr) = build(&root, &["--fix"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("no performance lints to fix"), "{stdout}");
    assert_eq!(std::fs::read(&main).unwrap(), pre);
    assert!(!root.join("main.rnx.bak").exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn fix_rolls_back_byte_identical_on_receipt_failure() {
    let root = proj("rollback", &rollback_case());
    let main = root.join("main.rnx");
    let bak = root.join("main.rnx.bak");
    let pre = std::fs::read(&main).unwrap();
    let (code, stdout, stderr) = build(&root, &["--fix"]);
    assert_eq!(code, 1, "{stdout}\n{stderr}");
    assert!(stderr.contains("main.rnx"), "{stderr}");
    assert!(stderr.contains("restored from backup"), "{stderr}");
    assert_eq!(std::fs::read(&main).unwrap(), pre);
    assert!(bak.exists(), "backup must be preserved on failure");
    assert_eq!(std::fs::read(&bak).unwrap(), pre);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn fix_refuses_when_backup_exists_unless_forced() {
    let root = proj("bakexists", &garbage());
    let main = root.join("main.rnx");
    std::fs::write(root.join("main.rnx.bak"), "stale").unwrap();
    let pre = std::fs::read(&main).unwrap();
    let (code, _, stderr) = build(&root, &["--fix"]);
    assert_eq!(code, 1, "{stderr}");
    assert!(stderr.contains(".bak"), "{stderr}");
    assert_eq!(std::fs::read(&main).unwrap(), pre);
    let (code, _, stderr) = build(&root, &["--fix", "--fix-force"]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(std::fs::read_to_string(&main).unwrap(), expected_fixed());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn fix_with_deny_on_same_lint_is_cli_error() {
    let root = proj("denycombo", &garbage());
    let (code, _, stderr) = build(&root, &["--fix", "--deny-perf-lints=P201"]);
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("--fix"), "{stderr}");
    let (code, _, stderr) = build(&root, &["--fix", "--deny-perf-lints"]);
    assert_eq!(code, 2, "{stderr}");
    let (code, _, stderr) = build(&root, &["--fix", "--deny-perf-lints=P202"]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(
        std::fs::read_to_string(root.join("main.rnx")).unwrap(),
        expected_fixed()
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn fix_on_clean_reports_nothing_to_do() {
    let root = proj("nothingtodo", &expected_fixed());
    let main = root.join("main.rnx");
    let pre = std::fs::read(&main).unwrap();
    let (code, stdout, stderr) = build(&root, &["--fix"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("no performance lints to fix"), "{stdout}");
    assert!(stdout.contains("artifact: "), "{stdout}");
    assert_eq!(std::fs::read(&main).unwrap(), pre);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn normal_builds_byte_identical_without_fix_flag() {
    for (tag, src) in [
        ("plain-a", expected_fixed()),
        ("plain-b", HELLO.to_string()),
    ] {
        let first = proj(&format!("{tag}-first"), &src);
        let second = proj(&format!("{tag}-second"), &src);
        let (code, stdout, stderr) = build(&first, &[]);
        assert_eq!(code, 0, "{stderr}");
        let a = artifact(&stdout);
        let (code, stdout, stderr) = build(&second, &[]);
        assert_eq!(code, 0, "{stderr}");
        let b = artifact(&stdout);
        assert_eq!(sha256(Path::new(&a)), sha256(Path::new(&b)), "{tag}");
        for dir in [&first, &second] {
            assert!(!dir.join("main.rnx.bak").exists());
            assert_eq!(std::fs::read_to_string(dir.join("main.rnx")).unwrap(), src);
        }
        let _ = std::fs::remove_dir_all(&first);
        let _ = std::fs::remove_dir_all(&second);
    }
}
