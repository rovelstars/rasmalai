#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::path::{Path, PathBuf};
use std::process::Command;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .expect("repo root")
}

fn stage_prelude(dest: &Path) {
    let out = Command::new(rnx())
        .args(["vendor", "@std/prelude", "--dir"])
        .arg(dest)
        .current_dir(repo())
        .env("RNX_VENDOR_DIR", repo().join("compiler/stdlib/src"))
        .env("NO_COLOR", "1")
        .output()
        .expect("run rnx vendor");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn write_hello(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("Project.config"),
        "export default {\n    project: { name: \"hello\", version: \"0.1.0\" },\n    entries: { main: \"lib.rnx\" },\n}\n",
    )
    .unwrap();
    std::fs::write(dir.join("lib.rnx"), "pub fn hi(): Int {\n    return 7\n}\n").unwrap();
}

fn run_project(dir: &Path, vendor: Option<&Path>) -> (bool, String, String) {
    let mut cmd = Command::new(rnx());
    cmd.args(["run", "--backend", "interpreter"])
        .current_dir(dir)
        .env_remove("RNX_VENDOR_DIR")
        .env("RNX_CACHE_HOME", dir.join("empty-cache"))
        .env("RNX_REGISTRY", "http://127.0.0.1:9")
        .env("NO_COLOR", "1");
    if let Some(vendor) = vendor {
        cmd.env("RNX_VENDOR_DIR", vendor);
    }
    let out = cmd.output().expect("run rnx");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn path_vendor_patch_resolves_offline() {
    let dir = scratch::dir("patch-vendor-path");
    let vendor = dir.join("vendor");
    stage_prelude(&vendor);
    write_hello(&vendor.join("hello"));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Project.config"),
        "export default {\n    project: { name: \"patch-path\", version: \"0.1.0\" },\n    entries: { main: \"src/main.rnx\" },\n    dependencies: { hello: \"0.1.0\" },\n    patch: { vendor: { path: \"vendor\" } },\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src").join("main.rnx"),
        "import { hi } from \"hello\"\n\nfn Main(): Int {\n    print(\"patch-path-ok\", hi())\n    return 0\n}\n",
    )
    .unwrap();
    let (ok, stdout, stderr) = run_project(&dir, None);
    assert!(ok, "{stderr}{stdout}");
    assert!(stdout.contains("patch-path-ok 7"), "{stdout}");
    assert!(
        stderr.contains(&format!("{}/@std/prelude", vendor.display())),
        "{stderr}"
    );
    assert!(
        stderr.contains(&format!("{}/hello", vendor.display())),
        "{stderr}"
    );
}

#[test]
fn git_vendor_patch_resolves_offline() {
    let dir = scratch::dir("patch-vendor-git");
    let source = dir.join("source");
    write_hello(&source.join("hello"));
    let status = Command::new("git")
        .args(["init", "-q"])
        .current_dir(&source)
        .status()
        .expect("git init");
    assert!(status.success());
    for (key, value) in [("user.email", "test@example.com"), ("user.name", "test")] {
        let status = Command::new("git")
            .args(["config", key, value])
            .current_dir(&source)
            .status()
            .expect("git config");
        assert!(status.success());
    }
    let status = Command::new("git")
        .args(["add", "."])
        .current_dir(&source)
        .status()
        .expect("git add");
    assert!(status.success());
    let status = Command::new("git")
        .args(["commit", "-qm", "hello"])
        .current_dir(&source)
        .status()
        .expect("git commit");
    assert!(status.success());
    let rev = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&source)
        .output()
        .expect("git rev-parse");
    let rev = String::from_utf8(rev.stdout).unwrap().trim().to_string();
    let prelude = dir.join("std");
    stage_prelude(&prelude);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Project.config"),
        format!(
            "export default {{\n    project: {{ name: \"patch-git\", version: \"0.1.0\" }},\n    entries: {{ main: \"src/main.rnx\" }},\n    dependencies: {{ hello: \"0.1.0\" }},\n    patch: {{ vendor: {{ git: \"file://{}\", rev: \"{rev}\" }} }},\n}}\n",
            source.display()
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("src").join("main.rnx"),
        "import { hi } from \"hello\"\n\nfn Main(): Int {\n    print(\"patch-git-ok\", hi())\n    return 0\n}\n",
    )
    .unwrap();
    let (ok, stdout, stderr) = run_project(&dir, Some(&prelude));
    assert!(ok, "{stderr}{stdout}");
    assert!(stdout.contains("patch-git-ok 7"), "{stdout}");
    assert!(stderr.contains(".rnx-cache/patches/git"), "{stderr}");
    assert!(stderr.contains("/hello"), "{stderr}");
}
