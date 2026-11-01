#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::process::Command;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn run(src: &str) -> std::process::Output {
    let dir = crate::scratch::dir("named-init");
    std::fs::write(
        dir.join("Project.config"),
        "export default {\n    project: { name: \"ni\", version: \"0.1.0\" },\n    entries: { main: \"main.rnx\" },\n}\n",
    )
    .unwrap();
    std::fs::write(dir.join("main.rnx"), src).unwrap();
    Command::new(rnx())
        .args(["run", "--backend", "interpreter"])
        .current_dir(&dir)
        .env("NO_COLOR", "1")
        .env(
            "RNX_VENDOR_DIR",
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../stdlib/src"),
        )
        .output()
        .unwrap()
}

fn stdout_of(src: &str) -> String {
    let out = run(src);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "{stderr}{stdout}");
    stdout.trim().to_string()
}

fn error_of(src: &str) -> String {
    let out = run(src);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(!out.status.success(), "expected failure: {stdout}{stderr}");
    format!("{stdout}{stderr}")
}

#[test]
fn struct_fields_bind_by_name_not_position() {
    let out = stdout_of(
        "struct P { let a: Int let b: Int }\nfn Main(): Int { let p = P(b: 1, a: 2) print(p.a, p.b) return 0 }\n",
    );
    assert_eq!(out, "2 1");
}

#[test]
fn class_fields_bind_by_name_through_new() {
    let out = stdout_of(
        "class C { let a: Int let b: Int }\nfn Main(): Int { let c = new C(b: 1, a: 2) print(c.a, c.b) return 0 }\n",
    );
    assert_eq!(out, "2 1");
}

#[test]
fn positional_then_named_still_maps_by_field() {
    let out = stdout_of(
        "struct P { let a: Int let b: Int let c: Int }\nfn Main(): Int { let p = P(1, c: 3, b: 2) print(p.a, p.b, p.c) return 0 }\n",
    );
    assert_eq!(out, "1 2 3");
}

#[test]
fn missing_field_is_reported() {
    let text = error_of(
        "struct P { let a: Int let b: Int }\nfn Main(): Int { let p = P(a: 1) return 0 }\n",
    );
    assert!(text.contains("missing field `b`"), "{text}");
}

#[test]
fn unknown_field_is_reported() {
    let text = error_of(
        "struct P { let a: Int let b: Int }\nfn Main(): Int { let p = P(a: 1, z: 2) return 0 }\n",
    );
    assert!(text.contains("has no field `z`"), "{text}");
}

#[test]
fn duplicate_field_is_reported() {
    let text = error_of(
        "struct P { let a: Int let b: Int }\nfn Main(): Int { let p = P(a: 1, a: 2) return 0 }\n",
    );
    assert!(text.contains("set twice"), "{text}");
}
