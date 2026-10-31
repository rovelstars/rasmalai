#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::process::Command;

#[test]
fn single_file_check_does_not_parse_unrelated_siblings() {
    let dir = crate::scratch::dir("check-sibling");
    std::fs::write(dir.join("broken.rnx"), "/*").unwrap();
    std::fs::write(dir.join("aaa.rnx"), "/*").unwrap();

    let deep = dir.join("sub").join("deeper");
    std::fs::create_dir_all(&deep).unwrap();
    let crash = format!("fn D(): Int {{ return {}true }}\n", "!".repeat(5000));
    std::fs::write(deep.join("crash.rnx"), crash).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_rnx"))
        .arg("check")
        .arg(dir.join("broken.rnx"))
        .env("NO_COLOR", "1")
        .output()
        .unwrap();

    assert_eq!(
        out.status.code(),
        Some(1),
        "the check must report the entry diagnostic instead of dying: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stderr);
    assert!(
        text.contains("unterminated block comment"),
        "expected the entry's own diagnostic, got: {text}"
    );
    assert!(
        text.contains("broken.rnx"),
        "diagnostic must name the requested entry, got: {text}"
    );
    assert!(
        !text.contains("aaa.rnx"),
        "single-file check must not attribute to a sibling, got: {text}"
    );
}

#[test]
fn project_mode_contains_crashing_sibling() {
    let dir = crate::scratch::dir("check-project-crash");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Project.config"),
        "export default {\n    project: {\n        name: \"sib\",\n        version: \"0.1.0\"\n    }\n}\n",
    )
    .unwrap();
    let entry = "fn Main(): Int { return 1 + }\n";
    std::fs::write(dir.join("src").join("main.rnx"), entry).unwrap();
    std::fs::write(dir.join("src").join("aaa.rnx"), entry).unwrap();
    std::fs::write(dir.join("src").join("broken.rnx"), "/*").unwrap();
    std::fs::write(
        dir.join("src").join("crash.rnx"),
        format!("fn D(): Int {{ return {}true }}\n", "!".repeat(5000)),
    )
    .unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_rnx"))
        .arg("check")
        .arg(dir.join("src").join("main.rnx"))
        .env("NO_COLOR", "1")
        .output()
        .unwrap();

    assert_eq!(
        out.status.code(),
        Some(1),
        "a crashing sibling must not abort the entry diagnostic: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stderr);
    assert!(
        text.contains("expected expression"),
        "expected the entry's parse diagnostic, got: {text}"
    );
    assert!(
        text.contains("main.rnx"),
        "entry diagnostic lost its attribution, got: {text}"
    );
    assert!(
        !text.contains("aaa.rnx"),
        "a sibling with the same span must not take attribution, got: {text}"
    );
    assert!(
        !text.contains("overflowed its stack") && !text.contains("panicked"),
        "crashing sibling escaped containment: {text}"
    );
}

#[test]
fn project_mode_still_discovers_siblings_for_attribution() {
    let dir = crate::scratch::dir("check-project-discovery");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Project.config"),
        "export default {\n    project: {\n        name: \"sib\",\n        version: \"0.1.0\"\n    }\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src").join("main.rnx"),
        "import { helper } from \"./util\";\nfn Main(): Int { return helper() }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src").join("util.rnx"),
        "fn helper(): Int { return 1 + }\n",
    )
    .unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_rnx"))
        .arg("check")
        .arg(dir.join("src").join("main.rnx"))
        .env("NO_COLOR", "1")
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1));
    let text = String::from_utf8_lossy(&out.stderr);
    assert!(
        text.contains("util.rnx"),
        "project mode must keep attributing imported-file diagnostics, got: {text}"
    );
}
