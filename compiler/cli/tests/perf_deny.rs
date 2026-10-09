use std::process::Command;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn proj(tag: &str, src: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("rnx-perfdeny-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("main.rnx"), src).unwrap();
    root
}

fn build(root: &std::path::Path, args: &[&str], env: &[(&str, &str)]) -> (i32, String, String) {
    let mut cmd = Command::new(rnx());
    cmd.arg("build")
        .arg("--no-color")
        .arg(root.join("main.rnx"));
    for a in args {
        cmd.arg(a);
    }
    cmd.env_remove("RNX_DENY_PERF_LINTS");
    cmd.env_remove("RNX_SHOW_PERF_NOTES");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

fn sha256(path: &str) -> String {
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

const CLEAN: &str = "fn Main(): Int {\n    let n = 16;\n    let a: Array<Int> = [];\n    let b: Array<Int> = [];\n    let c: Array<Int> = [];\n    let i = 0;\n    while i < n * n {\n        a.push(0);\n        b.push(0);\n        c.push(0);\n        i = i + 1;\n    }\n    let x = 0;\n    while x < n {\n        let y = 0;\n        while y < n {\n            let s = 0;\n            let z = 0;\n            while z < n {\n                s = s + a[x * n + z] * b[z * n + y];\n                z = z + 1;\n            }\n            c[x * n + y] = s;\n            y = y + 1;\n        }\n        x = x + 1;\n    }\n    let total = 0;\n    let t = 0;\n    while t < n * n {\n        total = total + c[t];\n        t = t + 1;\n    }\n    return total;\n}\n";

const ALIASED: &str = "fn Main(): Int {\n    let n = 16;\n    let a: Array<Int> = [];\n    let b: Array<Int> = [];\n    let c: Array<Int> = [];\n    let i = 0;\n    while i < n * n {\n        a.push(0);\n        b.push(0);\n        c.push(0);\n        i = i + 1;\n    }\n    let x = 0;\n    while x < n {\n        let y = 0;\n        while y < n {\n            let s = 0;\n            let z = 0;\n            while z < n {\n                s = s + a[x * n + z] * b[z * n + y];\n                z = z + 1;\n            }\n            a[x * n + y] = s;\n            y = y + 1;\n        }\n        x = x + 1;\n    }\n    let total = 0;\n    let t = 0;\n    while t < n * n {\n        total = total + c[t];\n        t = t + 1;\n    }\n    return total;\n}\n";

const MIXED: &str = "fn Clean(): Int {\n    let n = 16;\n    let a: Array<Int> = [];\n    let b: Array<Int> = [];\n    let c: Array<Int> = [];\n    let i = 0;\n    while i < n * n {\n        a.push(0);\n        b.push(0);\n        c.push(0);\n        i = i + 1;\n    }\n    let x = 0;\n    while x < n {\n        let y = 0;\n        while y < n {\n            let s = 0;\n            let z = 0;\n            while z < n {\n                s = s + a[x * n + z] * b[z * n + y];\n                z = z + 1;\n            }\n            c[x * n + y] = s;\n            y = y + 1;\n        }\n        x = x + 1;\n    }\n    return c[0];\n}\nfn Main(): Int {\n    let n = 16;\n    let a: Array<Int> = [];\n    let b: Array<Int> = [];\n    let c: Array<Int> = [];\n    let i = 0;\n    while i < n * n {\n        a.push(0);\n        b.push(0);\n        c.push(0);\n        i = i + 1;\n    }\n    let x = 0;\n    while x < n {\n        let y = 0;\n        while y < n {\n            let s = 0;\n            let z = 0;\n            while z < n {\n                s = s + a[x * n + z] * b[z * n + y];\n                z = z + 1;\n            }\n            a[x * n + y] = s;\n            y = y + 1;\n        }\n        x = x + 1;\n    }\n    return Clean() + c[0];\n}\n";

#[test]
fn unknown_deny_code_is_hard_error() {
    let root = proj("unknown", CLEAN);
    let (code, _, stderr) = build(&root, &["--deny-perf-lints=P999"], &[]);
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("P201, P202, P203"), "{stderr}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn deny_fires_only_on_denied_warns() {
    let root = proj("fires", ALIASED);
    let (code, _, stderr) = build(&root, &[], &[]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stderr.contains("warning[P201]"), "{stderr}");
    let (code, _, stderr) = build(&root, &["--deny-perf-lints"], &[]);
    assert_eq!(code, 1, "{stderr}");
    assert!(stderr.contains("error[P201]"), "{stderr}");
    assert!(stderr.contains("--deny-perf-lints"), "{stderr}");
    let (code, _, _) = build(&root, &["--deny-perf-lints=P202"], &[]);
    assert_eq!(code, 0);
    let (code, _, stderr) = build(&root, &["--deny-perf-lints=P201", "--deny-perf-lints=P203"], &[]);
    assert_eq!(code, 1, "{stderr}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn deny_env_mirror_and_cli_precedence() {
    let root = proj("env", ALIASED);
    let (code, _, stderr) = build(&root, &[], &[("RNX_DENY_PERF_LINTS", "P201")]);
    assert_eq!(code, 1, "{stderr}");
    let (code, _, stderr) = build(&root, &[], &[("RNX_DENY_PERF_LINTS", "P201,P203")]);
    assert_eq!(code, 1, "{stderr}");
    let (code, _, _) = build(&root, &[], &[("RNX_DENY_PERF_LINTS", "P202")]);
    assert_eq!(code, 0);
    let (code, _, stderr) = build(&root, &["--deny-perf-lints"], &[("RNX_DENY_PERF_LINTS", "P999")]);
    assert_eq!(code, 1, "cli flag overrides env: {stderr}");
    let (code, _, stderr) = build(&root, &[], &[("RNX_DENY_PERF_LINTS", "P999")]);
    assert_eq!(code, 2, "{stderr}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn deny_flag_leaves_binary_bytes_identical() {
    let plain_root = proj("bytes-plain", CLEAN);
    let denied_root = proj("bytes-denied", CLEAN);
    let (code, stdout, stderr) = build(&plain_root, &[], &[]);
    assert_eq!(code, 0, "{stderr}");
    assert!(!stderr.contains("note[P1"), "{stderr}");
    let plain = artifact(&stdout);
    let (code, stdout, stderr) = build(&denied_root, &["--deny-perf-lints"], &[]);
    assert_eq!(code, 0, "{stderr}");
    assert!(!stderr.contains("note[P1"), "{stderr}");
    let denied = artifact(&stdout);
    assert_eq!(sha256(&plain), sha256(&denied));
    let (code, stdout, stderr) = build(&plain_root, &["--show-perf-notes"], &[]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stderr.contains("note[P101]"), "{stderr}");
    let shown = artifact(&stdout);
    assert_eq!(sha256(&plain), sha256(&shown));
    let _ = std::fs::remove_dir_all(&plain_root);
    let _ = std::fs::remove_dir_all(&denied_root);
}

#[test]
fn perf_notes_hidden_by_default_and_restored_with_flag() {
    let root = proj("notegate", CLEAN);
    let (code, _, default_err) = build(&root, &[], &[]);
    assert_eq!(code, 0, "{default_err}");
    assert!(
        !default_err.contains("note[P1"),
        "default output must have zero Tier-1 lines:\n{default_err}"
    );
    let (code, _, shown_err) = build(&root, &["--show-perf-notes"], &[]);
    assert_eq!(code, 0, "{shown_err}");
    assert!(shown_err.contains("note[P101]"), "{shown_err}");
    assert!(shown_err.contains("main.rnx:"), "{shown_err}");
    let default_lints: Vec<&str> = default_err
        .lines()
        .filter(|l| l.contains("note[") || l.contains("warning["))
        .collect();
    let shown_lints: Vec<&str> = shown_err
        .lines()
        .filter(|l| l.contains("note[") || l.contains("warning["))
        .collect();
    assert!(default_lints.is_empty(), "{default_err}");
    assert!(!shown_lints.is_empty(), "{shown_err}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn show_notes_env_mirror_and_cli_precedence() {
    let root = proj("shownote-env", CLEAN);
    let (code, _, stderr) = build(&root, &[], &[("RNX_SHOW_PERF_NOTES", "1")]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stderr.contains("note[P101]"), "{stderr}");
    let (code, _, stderr) = build(&root, &[], &[("RNX_SHOW_PERF_NOTES", "0")]);
    assert_eq!(code, 0, "{stderr}");
    assert!(!stderr.contains("note[P1"), "{stderr}");
    let (code, _, stderr) = build(&root, &["--show-perf-notes"], &[("RNX_SHOW_PERF_NOTES", "0")]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stderr.contains("note[P101]"), "{stderr}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn deny_and_show_compose() {
    let root = proj("compose", MIXED);
    let (code, _, stderr) = build(&root, &["--deny-perf-lints", "--show-perf-notes"], &[]);
    assert_eq!(code, 1, "{stderr}");
    assert!(stderr.contains("error[P201]"), "{stderr}");
    assert!(stderr.contains("note[P101]"), "{stderr}");
    assert!(stderr.contains("--deny-perf-lints"), "{stderr}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn deny_notes_never_promote() {
    let root = proj("nopromote", CLEAN);
    let (code, _, stderr) = build(&root, &["--deny-perf-lints", "--show-perf-notes"], &[]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stderr.contains("note[P101]"), "{stderr}");
    assert!(!stderr.contains("error[P101]"), "{stderr}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn allow_attribute_suppresses_deny() {
    let src = ALIASED.replacen(
        "fn Main(): Int {",
        "#[Allow(PerfP201)]\nfn Main(): Int {",
        1,
    );
    let root = proj("allow", &src);
    let (code, _, stderr) = build(&root, &["--deny-perf-lints"], &[]);
    assert_eq!(code, 0, "{stderr}");
    assert!(!stderr.contains("P201"), "{stderr}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn relative_entry_path_renders_notes_only_with_flag() {
    let root = proj("relpath", CLEAN);
    let out = Command::new(rnx())
        .arg("build")
        .arg("--no-color")
        .arg("main.rnx")
        .current_dir(&root)
        .env_remove("RNX_DENY_PERF_LINTS")
        .env_remove("RNX_SHOW_PERF_NOTES")
        .output()
        .unwrap();
    assert_eq!(
        out.status.code().unwrap(),
        0,
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(!stderr.contains("note[P1"), "{stderr}");
    let out = Command::new(rnx())
        .arg("build")
        .arg("--no-color")
        .arg("--show-perf-notes")
        .arg("main.rnx")
        .current_dir(&root)
        .env_remove("RNX_DENY_PERF_LINTS")
        .env_remove("RNX_SHOW_PERF_NOTES")
        .output()
        .unwrap();
    assert_eq!(
        out.status.code().unwrap(),
        0,
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("note[P101]"), "{stderr}");
    assert!(stderr.contains("main.rnx:"), "{stderr}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn deny_rejected_for_lib_and_emit_obj() {
    let root = proj("modes", CLEAN);
    let (code, _, stderr) = build(&root, &["--deny-perf-lints", "--lib"], &[]);
    assert_eq!(code, 2, "{stderr}");
    let (code, _, stderr) = build(&root, &["--deny-perf-lints", "--emit-obj"], &[]);
    assert_eq!(code, 2, "{stderr}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn lint_stream_matches_dev_and_release() {
    let root = proj("parity", CLEAN);
    let (_, _, dev_err) = build(&root, &["--show-perf-notes"], &[]);
    let (_, _, rel_err) = build(&root, &["--show-perf-notes", "--release"], &[]);
    let dev_lints: Vec<&str> = dev_err
        .lines()
        .filter(|l| l.contains("note[") || l.contains("warning["))
        .collect();
    let rel_lints: Vec<&str> = rel_err
        .lines()
        .filter(|l| l.contains("note[") || l.contains("warning["))
        .collect();
    assert!(!dev_lints.is_empty(), "{dev_err}");
    assert_eq!(dev_lints, rel_lints, "{dev_err}\n---\n{rel_err}");
    let (_, _, default_err) = build(&root, &[], &[]);
    assert!(
        !default_err.lines().any(|l| l.contains("note[P1")),
        "{default_err}"
    );
    let _ = std::fs::remove_dir_all(&root);
}
