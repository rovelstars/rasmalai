#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn run(args: &[&str], dir: &std::path::Path) -> (i32, String) {
    let out = std::process::Command::new(rnx())
        .args(args)
        .current_dir(dir)
        .env_remove("RNX_REGISTRY")
        .env_remove("RNX_CACHE_HOME")
        .output()
        .unwrap();
    let code = out.status.code().unwrap();
    let text =
        String::from_utf8(out.stdout).unwrap() + &String::from_utf8(out.stderr).unwrap();
    (code, text)
}

fn pkg(tag: &str, main: &str) -> crate::scratch::Scratch {
    let dir = crate::scratch::dir(&format!("rtcli-{tag}"));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Project.config"),
        "export default {\n    project: {\n        name: \"probe\",\n        version: \"0.1.0\",\n        description: \"Probe package\"\n    }\n}\n",
    )
    .unwrap();
    std::fs::write(dir.join("LICENSE"), "MIT License\n").unwrap();
    std::fs::write(dir.join("src").join("main.rnx"), main).unwrap();
    dir
}

#[test]
fn audit_rejects_bare_rnx_reference() {
    let dir = pkg(
        "audit-bad",
        "fn Main(): Int {\n    return __rnx_clock_mono();\n}\n",
    );
    let (code, text) = run(&["audit", "--path", &dir.to_string_lossy()], &dir);
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("__rnx_clock_mono"), "{text}");
    assert!(text.contains("undefined variable"), "{text}");
    assert!(text.contains("@std/*"), "{text}");
}

#[test]
fn audit_rejects_declared_runtime_import_in_user_code() {
    let dir = pkg(
        "audit-rt",
        "import { fn __rnx_clock_mono(): Int } from runtime \"time\"\nfn Main(): Int {\n    return __rnx_clock_mono();\n}\n",
    );
    let (code, text) = run(&["audit", "--path", &dir.to_string_lossy()], &dir);
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("__rnx_clock_mono"), "{text}");
    assert!(text.contains("@std/time"), "{text}");
}

#[test]
fn audit_passes_without_bare_rnx_references() {
    let dir = pkg("audit-good", "fn Main(): Int {\n    return 0;\n}\n");
    let (code, text) = run(&["audit", "--path", &dir.to_string_lossy()], &dir);
    assert_eq!(code, 0, "{text}");
    assert!(!text.contains("__rnx_"), "{text}");
}

#[test]
fn audit_passes_for_declared_native_alias() {
    let dir = pkg(
        "audit-alias",
        "import { fn clock(): Int as __rnx_clock } from native \"rt\"\nunsafe fn tick(): Int {\n    return __rnx_clock();\n}\nfn Main(): Int {\n    unsafe {\n        return tick();\n    }\n}\n",
    );
    let (code, text) = run(&["audit", "--path", &dir.to_string_lossy()], &dir);
    assert_eq!(code, 0, "{text}");
    assert!(!text.contains("__rnx_"), "{text}");
}

#[test]
fn publish_precheck_rejects_runtime_import_before_network() {
    let dir = pkg(
        "publish-bad",
        "import { fn __rnx_clock_mono(): Int } from runtime \"time\"\nfn Main(): Int {\n    return __rnx_clock_mono();\n}\n",
    );
    let (code, text) = run(
        &[
            "publish",
            "--registry",
            "http://127.0.0.1:9",
            "--token",
            "probe-token",
        ],
        &dir,
    );
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("__rnx_clock_mono"), "{text}");
    assert!(!text.contains("cannot reach registry"), "{text}");
}

#[test]
fn publish_precheck_accepts_cross_file_conformance_before_network() {
    let dir = pkg(
        "publish-crossfile",
        "import {\n    Pager,\n    Page\n} from \"./types\";\n\nexport fn first(): Pager {\n    return new Page();\n}\n\nfn Main(): Int {\n    return first().next();\n}\n",
    );
    std::fs::write(
        dir.join("src").join("types.rnx"),
        "export interface Pager {\n    fn next(): Int;\n}\n\nexport class Page with Pager {\n    fn next(): Int {\n        return 1;\n    }\n}\n",
    )
    .unwrap();
    let (code, text) = run(
        &[
            "publish",
            "--registry",
            "http://127.0.0.1:9",
            "--token",
            "probe-token",
        ],
        &dir,
    );
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("publish failed"), "{text}");
    assert!(!text.contains("declare runtime internals"), "{text}");
}

#[test]
fn publish_precheck_rejects_bare_rnx_before_network() {
    let dir = pkg(
        "publish-bare",
        "fn Main(): Int {\n    return __rnx_clock_mono();\n}\n",
    );
    let (code, text) = run(
        &[
            "publish",
            "--registry",
            "http://127.0.0.1:9",
            "--token",
            "probe-token",
        ],
        &dir,
    );
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("__rnx_clock_mono"), "{text}");
    assert!(!text.contains("cannot reach registry"), "{text}");
}
