use std::path::PathBuf;

fn write_project(tag: &str, name: &str, version: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-pub-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Project.config"),
        format!("export default {{\n    project: {{\n        name: \"{name}\",\n        version: \"{version}\",\n        description: \"T.\"\n    }}\n}}\n"),
    )
    .unwrap();
    std::fs::write(
        dir.join("src").join("main.rnx"),
        "/**Adds one.*/\npub fn add_one(x: Int): Int { return x + 1; }\nfn Main(): Int { return 0; }\n",
    )
    .unwrap();
    dir
}

fn rnx(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_rnx"));
    cmd.args(args).current_dir(dir);
    cmd.output().unwrap()
}

fn gunzip(data: &[u8]) -> Vec<u8> {
    frontend::gzip::decompress_gzip(data).expect("valid gzip")
}

#[test]
fn test_pack_embeds_no_generated_doc_json() {
    let dir = write_project("doc", "probe", "1.2.3");
    let out = rnx(&dir, &["pack", "-z"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let gz = dir.join("target").join("package").join("probe-1.2.3.tar.gz");
    assert!(gz.is_file(), "archive missing");
    let tar = gunzip(&std::fs::read(&gz).unwrap());
    let text = String::from_utf8_lossy(&tar);
    assert!(!text.contains(".rnx/doc.json"), "generated doc.json still embedded");
    assert!(text.contains("Project.config"), "config missing from archive");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_pack_rejects_bad_manifest() {
    let dir = write_project("bad", "Bad_Name", "1.2.3");
    let out = rnx(&dir, &["pack", "-z"]);
    assert!(!out.status.success(), "upper-case name accepted");
    let err = String::from_utf8_lossy(&out.stdout).into_owned()
        + &String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(err.contains("invalid package name"), "{err}");

    let dir = write_project("badver", "probe", "1.2");
    let out = rnx(&dir, &["pack", "-z"]);
    assert!(!out.status.success(), "non-semver accepted");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_publish_requires_token() {
    let dir = write_project("tok", "probe", "1.2.3");
    let out = rnx(&dir, &["publish", "--registry", "http://localhost:9/api/packages"]);
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(stdout.contains("error[E501]"), "{stdout}");
    assert!(stdout.contains("RNX_TOKEN"), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}
