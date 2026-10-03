const EXPECTED: &str = "Multi-package resolution success: 42\n";

fn fixture_tree(tag: &str) -> std::path::PathBuf {
    let base = std::env::temp_dir().join(format!("rnx-deplock-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let src = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/multi_package");
    copy_dir(&src.join("app"), &base.join("app"));
    copy_dir(&src.join("dep_math"), &base.join("dep_math"));
    base.join("app")
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), &dest).unwrap();
        }
    }
}

fn lock(app: &std::path::Path) {
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let out = std::process::Command::new(rnx)
        .arg("lock")
        .current_dir(app)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "Locked 2 packages to Project.deplock\n"
    );
}

fn valid_hex64(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

#[test]
fn test_rnx_lock_generation() {
    let app = fixture_tree("gen");
    lock(&app);
    let text = std::fs::read_to_string(app.join("Project.deplock")).unwrap();
    assert!(text.contains("version: 2"), "{text}");
    let app_pos = text.find("name: \"app\"").unwrap();
    let dep_pos = text.find("name: \"dep_math\"").unwrap();
    assert!(app_pos < dep_pos, "packages sorted by name:\n{text}");
    assert!(text.contains("source: \"root\""), "{text}");
    assert!(text.contains("source: \"path:../dep_math\""), "{text}");
    assert!(text.contains("dependencies: [\"dep_math 0.1.0\"]"), "{text}");
    let checks: Vec<&str> = text.lines().filter_map(|l| l.trim().strip_prefix("checksum: \"")).collect();
    assert_eq!(checks.len(), 2);
    for c in checks {
        let hex = c.strip_suffix(",").unwrap_or(c).strip_suffix('"').unwrap_or(c);
        assert!(valid_hex64(hex), "{c}");
    }
    let again = std::fs::read_to_string(app.join("Project.deplock")).unwrap();
    lock(&app);
    assert_eq!(std::fs::read_to_string(app.join("Project.deplock")).unwrap(), again);
    let _ = std::fs::remove_dir_all(app.parent().unwrap());
}

#[test]
fn test_locked_run_success() {
    let app = fixture_tree("run");
    lock(&app);
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let run = std::process::Command::new(rnx)
        .arg("run")
        .arg("--locked")
        .current_dir(&app)
        .output()
        .unwrap();
    assert_eq!(run.status.code().unwrap(), 42);
    assert_eq!(String::from_utf8(run.stdout).unwrap(), EXPECTED);
    let _ = std::fs::remove_dir_all(app.parent().unwrap());
}

#[test]
fn test_locked_checksum_tamper_detection() {
    let app = fixture_tree("tamper");
    lock(&app);
    let lib = app.parent().unwrap().join("dep_math").join("src").join("lib.rnx");
    let before = std::fs::read_to_string(&lib).unwrap();
    std::fs::write(&lib, before + "\n").unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let run = std::process::Command::new(rnx)
        .arg("run")
        .arg("--locked")
        .current_dir(&app)
        .output()
        .unwrap();
    assert!(!run.status.success());
    let text =
        String::from_utf8(run.stdout).unwrap() + &String::from_utf8(run.stderr).unwrap();
    assert!(text.contains("E108"), "{text}");
    assert!(text.contains("checksum mismatch"), "{text}");
    let _ = std::fs::remove_dir_all(app.parent().unwrap());
}

#[test]
fn test_missing_deplock_error() {
    let app = fixture_tree("missing");
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let run = std::process::Command::new(rnx)
        .arg("run")
        .arg("--locked")
        .current_dir(&app)
        .output()
        .unwrap();
    assert!(!run.status.success());
    let text =
        String::from_utf8(run.stdout).unwrap() + &String::from_utf8(run.stderr).unwrap();
    assert!(text.contains("Project.deplock not found"), "{text}");
    let _ = std::fs::remove_dir_all(app.parent().unwrap());
}
