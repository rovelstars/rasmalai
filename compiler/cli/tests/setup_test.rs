use std::path::PathBuf;

fn fresh_home(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-setup-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn setup_cmd(home: &std::path::Path, args: &[&str], path: Option<&std::path::Path>) -> std::process::Output {
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let mut cmd = std::process::Command::new(rnx);
    cmd.arg("setup");
    for a in args {
        cmd.arg(a);
    }
    cmd.env("HOME", home);
    cmd.env("XDG_CONFIG_HOME", home.join(".config"));
    if let Some(p) = path {
        cmd.env("PATH", p);
    }
    cmd.output().unwrap()
}

fn empty_path() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-setup-empty-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn setup_helix_writes_stanza_and_is_idempotent() {
    let home = fresh_home("helix");
    let out = setup_cmd(&home, &["helix"], None);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let target = home.join(".config").join("helix").join("languages.toml");
    let written = std::fs::read_to_string(&target).unwrap();
    let shipped = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("editors").join("helix").join("languages.toml"),
    )
    .unwrap();
    assert_eq!(written, shipped);
    let again = setup_cmd(&home, &["helix"], None);
    assert!(again.status.success(), "{}", String::from_utf8_lossy(&again.stderr));
    assert!(String::from_utf8(again.stdout).unwrap().contains("already configured"));
    assert_eq!(std::fs::read_to_string(&target).unwrap(), shipped);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn setup_helix_merges_with_existing_config() {
    let home = fresh_home("helix-merge");
    let target = home.join(".config").join("helix").join("languages.toml");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, "# my config\n").unwrap();
    let out = setup_cmd(&home, &["helix"], None);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let merged = std::fs::read_to_string(&target).unwrap();
    assert!(merged.starts_with("# my config"), "{merged}");
    assert!(merged.contains("name = \"rasmalai\""), "{merged}");
    assert_eq!(merged.matches("name = \"rasmalai\"").count(), 1);
    assert_eq!(std::fs::read_to_string(target.with_extension("toml.bak")).unwrap(), "# my config\n");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn setup_neovim_writes_plugin_and_is_idempotent() {
    let home = fresh_home("neovim");
    let out = setup_cmd(&home, &["neovim"], None);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let target = home.join(".config").join("nvim").join("lua").join("rasmalai.lua");
    let written = std::fs::read_to_string(&target).unwrap();
    let shipped = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("editors").join("neovim").join("rasmalai.lua"),
    )
    .unwrap();
    assert_eq!(written, shipped);
    assert!(String::from_utf8(out.stdout).unwrap().contains("require(\"rasmalai\")"));
    let again = setup_cmd(&home, &["neovim"], None);
    assert!(again.status.success(), "{}", String::from_utf8_lossy(&again.stderr));
    assert!(String::from_utf8(again.stdout).unwrap().contains("already configured"));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn setup_vscode_without_code_prints_manual_steps() {
    let home = fresh_home("vscode");
    let empty = empty_path();
    let out = setup_cmd(&home, &["vscode"], Some(&empty));
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("code --install-extension"), "{stdout}");
    assert!(stdout.contains("Marketplace"), "{stdout}");
    assert!(stdout.contains("rnx"), "{stdout}");
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&empty);
}

#[test]
fn setup_zed_prints_dev_extension_steps() {
    let home = fresh_home("zed");
    let out = setup_cmd(&home, &["zed"], None);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("Install Dev Extension"), "{stdout}");
    assert!(stdout.contains("rnx-lsp"), "{stdout}");
    assert!(!stdout.to_lowercase().contains("marketplace") || stdout.contains("no registry"), "{stdout}");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn setup_unknown_editor_fails_with_supported_list() {
    let home = fresh_home("unknown");
    let out = setup_cmd(&home, &["emacs"], None);
    assert!(!out.status.success());
    assert_eq!(out.status.code().unwrap(), 2);
    let stderr = String::from_utf8(out.stderr).unwrap();
    for name in ["vscode", "zed", "helix", "neovim"] {
        assert!(stderr.contains(name), "{stderr}");
    }
    let missing = setup_cmd(&home, &[], None);
    assert!(!missing.status.success());
    assert_eq!(missing.status.code().unwrap(), 2);
    let _ = std::fs::remove_dir_all(&home);
}
