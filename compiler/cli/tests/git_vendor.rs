const EXPECTED: &str = "Git dep success: 42\n";

fn git(args: &[&str], cwd: &std::path::Path) {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git invocation");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn base(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-git-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn bare_remote(dir: &std::path::Path) -> String {
    let work = dir.join("remote");
    std::fs::create_dir_all(work.join("src")).unwrap();
    std::fs::write(
        work.join("Project.config"),
        "export default {\n    project: {\n        name: \"glib\",\n        version: \"0.1.0\",\n        entry: \"src/lib.rnx\"\n    }\n}\n",
    )
    .unwrap();
    std::fs::write(
        work.join("src").join("lib.rnx"),
        "fn double(x: Int): Int {\n    return x * 2;\n}\n",
    )
    .unwrap();
    git(&["init", "-q"], &work);
    git(&["add", "-A"], &work);
    git(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init"], &work);
    git(&["tag", "v0.1.0"], &work);
    let bare = dir.join("remote.git");
    git(&["clone", "-q", "--bare", &work.to_string_lossy(), &bare.to_string_lossy()], dir);
    bare.to_string_lossy().into_owned()
}

fn app_with_git_dep(dir: &std::path::Path, remote: &str, rev: Option<&str>) -> std::path::PathBuf {
    let app = dir.join("app");
    std::fs::create_dir_all(app.join("src")).unwrap();
    let dep = match rev {
        Some(r) => format!("glib: {{ git: \"{remote}\", rev: \"{r}\" }}\n"),
        None => format!("glib: {{ git: \"{remote}\" }}\n"),
    };
    std::fs::write(
        app.join("Project.config"),
        format!("export default {{\n    project: {{\n        name: \"app\",\n        version: \"0.1.0\"\n    }},\n    dependencies: {{\n        {dep}    }}\n}}\n"),
    )
    .unwrap();
    std::fs::write(
        app.join("src").join("main.rnx"),
        "import { double } from \"glib\";\n\nfn Main(): Int {\n    let res = double(21);\n    print(\"Git dep success:\", res);\n    return 0;\n}\n",
    )
    .unwrap();
    app
}

fn rnx(args: &[&str], cwd: &std::path::Path) -> (i32, String) {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_rnx"))
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    let code = out.status.code().unwrap();
    let text =
        String::from_utf8(out.stdout).unwrap() + &String::from_utf8(out.stderr).unwrap();
    (code, text)
}

#[test]
fn test_git_dep_fetch_and_run() {
    let dir = base("fetch");
    let remote = bare_remote(&dir);
    let app = app_with_git_dep(&dir, &remote, Some("v0.1.0"));
    let (code, text) = rnx(&["fetch"], &app);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("Fetched glib (v0.1.0)"), "{text}");
    let cached = app.join(".rnx").join("cache").join("git").join("glib-v0.1.0");
    assert!(cached.join("Project.config").is_file(), "{}", cached.display());
    assert!(cached.join("src").join("lib.rnx").is_file());
    let (code, text) = rnx(&["run"], &app);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains(EXPECTED), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_rnx_vendor_offline() {
    let dir = base("vendor");
    let remote = bare_remote(&dir);
    let app = app_with_git_dep(&dir, &remote, Some("v0.1.0"));
    let (code, text) = rnx(&["vendor"], &app);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("Vendored 1 packages into vendor/"), "{text}");
    assert!(app.join("vendor").join("glib").join("src").join("lib.rnx").is_file());
    assert!(app.join("vendor").join("glib").join("Project.config").is_file());
    assert!(!app.join("vendor").join("glib").join(".git").exists());
    let _ = std::fs::remove_dir_all(dir.join("remote.git"));
    let _ = std::fs::remove_dir_all(dir.join("remote"));
    let _ = std::fs::remove_dir_all(app.join(".rnx"));
    let (code, text) = rnx(&["run"], &app);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains(EXPECTED), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_unpinned_git_dep_rejected() {
    let dir = base("unpinned");
    let remote = bare_remote(&dir);
    let app = app_with_git_dep(&dir, &remote, None);
    let (code, text) = rnx(&["run"], &app);
    assert_ne!(code, 0);
    assert!(text.contains("E108"), "{text}");
    assert!(text.contains("rev"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}
