use std::process::Command;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn check(dir: &std::path::Path, name: &str, src: &str) -> (bool, String) {
    let file = dir.join(format!("{name}.rnx"));
    std::fs::write(&file, src).unwrap();
    let out = Command::new(rnx())
        .arg("check")
        .arg(&file)
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

#[test]
fn persist_is_removed_and_field_attributes_are_checked() {
    let dir = std::env::temp_dir().join(format!("rnx-persist-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    for (name, src) in [
        (
            "field",
            "class Counter {\n    #[Persist] let count: Int = 0\n}\nfn Main(): Int { return 0 }\n",
        ),
        (
            "class",
            "#[Persist]\nclass Counter {}\nfn Main(): Int { return 0 }\n",
        ),
    ] {
        let (ok, text) = check(&dir, name, src);
        assert!(!ok, "{name} accepted #[Persist]:\n{text}");
        assert!(
            text.contains("unknown attribute `#[Persist]`"),
            "{name} did not name the attribute:\n{text}"
        );
    }

    let (ok, text) = check(
        &dir,
        "allow",
        "class Node {\n    #[Allow(CyclicReference)] let next: Node? = null\n    #[Inline(Always)] fn bump(): Int { return 1 }\n}\nfn Main(): Int { return 0 }\n",
    );
    assert!(ok, "valid field and method attributes rejected:\n{text}");

    let _ = std::fs::remove_dir_all(&dir);
}
