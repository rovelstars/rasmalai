use std::io::Write;
use std::process::{Command, Stdio};

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn fresh_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-secrun-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(path: &std::path::Path, content: &str) {
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(content.as_bytes()).unwrap();
}

fn run_rnx(main: &std::path::Path, backend: &str) -> std::process::Output {
    Command::new(rnx())
        .arg("run")
        .arg(main)
        .arg("--backend")
        .arg(backend)
        .arg("--no-color")
        .arg("-q")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn combined(out: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

const FILE_PRELUDE: &str = "import { File, OpenMode } from \"@std/fs\";\n";
const PROC_PRELUDE: &str = "import { Process, Stdio, SpawnOptions } from \"@std/process\";\nimport { Map } from \"@std/collections\";\n";

#[test]
fn s301_direct_write_rejected() {
    let dir = fresh_dir("direct");
    let main = dir.join("main.rnx");
    write(
        &main,
        &format!(
            "{FILE_PRELUDE}fn Main(): Int {{\n    let f = File.open(\"Project.deplock\", OpenMode.Write);\n    f.writeText(\"hacked\");\n    return 0;\n}}\n"
        ),
    );
    let out = run_rnx(&main, "interpreter");
    assert_ne!(out.status.code(), Some(0), "{}", combined(&out));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("panic [S301]"), "{stderr}");
    assert!(stderr.contains("Project.deplock"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn s301_traversal_rejected() {
    let dir = fresh_dir("traversal");
    let main = dir.join("main.rnx");
    write(
        &main,
        &format!(
            "{FILE_PRELUDE}fn Main(): Int {{\n    let f = File.open(\"./foo/../Project.config\", OpenMode.Write);\n    f.writeText(\"hacked\");\n    return 0;\n}}\n"
        ),
    );
    let out = run_rnx(&main, "interpreter");
    assert_ne!(out.status.code(), Some(0), "{}", combined(&out));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("panic [S301]"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn s301_vcs_tree_rejected() {
    let dir = fresh_dir("vcs");
    let main = dir.join("main.rnx");
    write(
        &main,
        &format!(
            "{FILE_PRELUDE}fn Main(): Int {{\n    let f = File.open(\".git/hooks/pre-commit\", OpenMode.Write);\n    f.writeText(\"hacked\");\n    return 0;\n}}\n"
        ),
    );
    let out = run_rnx(&main, "interpreter");
    assert_ne!(out.status.code(), Some(0), "{}", combined(&out));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("panic [S301]"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn s301_remove_rejected() {
    let dir = fresh_dir("remove");
    let main = dir.join("main.rnx");
    write(
        &main,
        "import fs from \"@std/fs\";\nfn Main(): Int {\n    fs.remove(\"Project.config\");\n    return 0;\n}\n",
    );
    let out = run_rnx(&main, "interpreter");
    assert_ne!(out.status.code(), Some(0), "{}", combined(&out));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("panic [S301]"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn s301_legit_write_allowed() {
    let dir = fresh_dir("legit");
    let main = dir.join("main.rnx");
    write(
        &main,
        &format!(
            "{FILE_PRELUDE}fn Main(): Int {{\n    let f = File.open(\"notes.txt\", OpenMode.Write);\n    f.writeText(\"fine\");\n    f.close();\n    return 0;\n}}\n"
        ),
    );
    let out = Command::new(rnx())
        .arg("run")
        .arg(&main)
        .arg("--backend")
        .arg("interpreter")
        .arg("--no-color")
        .arg("-q")
        .stdin(Stdio::null())
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", combined(&out));
    assert_eq!(std::fs::read_to_string(dir.join("notes.txt")).unwrap(), "fine");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn s301_direct_write_rejected_aot() {
    let dir = fresh_dir("aot");
    let main = dir.join("main.rnx");
    write(
        &main,
        &format!(
            "{FILE_PRELUDE}fn Main(): Int {{\n    let f = File.open(\"Project.deplock\", OpenMode.Write);\n    f.writeText(\"hacked\");\n    return 0;\n}}\n"
        ),
    );
    let bin = dir.join("s301_bin");
    let build = Command::new(rnx())
        .arg("build")
        .arg(&main)
        .arg("-o")
        .arg(&bin)
        .arg("-q")
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", combined(&build));
    let run = Command::new(&bin)
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_ne!(run.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(stderr.contains("panic [S301]"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn child_env_isolated_but_forwarding_works() {
    let dir = fresh_dir("env");
    let main = dir.join("main.rnx");
    write(
        &main,
        &format!(
            "{PROC_PRELUDE}fn Main(): Int {{\n    let probe = Process.run(\"sh\", [\"-c\", \"echo MARKER:$SECRET_CREDENTIAL:END\"]);\n    print(probe.stdoutText());\n    let env_map = new Map<String, String>();\n    env_map.set(\"RNX_KID_VAR\", \"kid_42\");\n    let opts = new SpawnOptions(null, env_map, Stdio.Piped, Stdio.Piped, Stdio.Piped);\n    let fwd = Process.run(\"sh\", [\"-c\", \"echo $RNX_KID_VAR\"], opts);\n    print(fwd.stdoutText());\n    return 0;\n}}\n"
        ),
    );
    let out = Command::new(rnx())
        .arg("run")
        .arg(&main)
        .arg("--backend")
        .arg("interpreter")
        .arg("--no-color")
        .arg("-q")
        .env("SECRET_CREDENTIAL", "super_secret")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", combined(&out));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains("super_secret"), "ambient leak: {stdout}");
    assert!(stdout.contains("MARKER::END"), "scrubbed child: {stdout}");
    assert!(stdout.contains("kid_42"), "explicit forward: {stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn s401_locked_spawn_denied_and_allowed() {
    let dir = fresh_dir("s401");
    let app = dir.join("app");
    std::fs::create_dir_all(app.join("src")).unwrap();
    write(&app.join("Project.config"), "export default {\n    project: {\n        name: \"app\",\n        version: \"0.1.0\"\n    }\n}\n");
    write(
        &app.join("src/main.rnx"),
        &format!(
            "{PROC_PRELUDE}fn Main(): Int {{\n    let out = Process.run(\"sh\", [\"-c\", \"echo hi\"]);\n    assert(out.exitCode == 0, \"child ran\");\n    return 0;\n}}\n"
        ),
    );
    let lock = Command::new(rnx())
        .arg("lock")
        .current_dir(&app)
        .output()
        .unwrap();
    assert!(lock.status.success(), "{}", combined(&lock));
    let lock_text = std::fs::read_to_string(app.join("Project.deplock")).unwrap();
    assert!(lock_text.contains("tier: \"hazard\""), "{lock_text}");
    assert!(lock_text.contains("sys:exec:sh"), "{lock_text}");
    let allowed = Command::new(rnx())
        .arg("run")
        .arg("--locked")
        .arg("--backend")
        .arg("interpreter")
        .arg("--no-color")
        .arg("-q")
        .current_dir(&app)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(allowed.status.code(), Some(0), "{}", combined(&allowed));
    let lock_path = app.join("Project.deplock");
    let stripped = lock_text.replace("capabilities: [\"sys:exec:sh\"]", "capabilities: []");
    write(&lock_path, &stripped);
    let denied = Command::new(rnx())
        .arg("run")
        .arg("--locked")
        .arg("--backend")
        .arg("interpreter")
        .arg("--no-color")
        .arg("-q")
        .current_dir(&app)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_ne!(denied.status.code(), Some(0));
    let denied_text = combined(&denied);
    assert!(denied_text.contains("S101"), "{denied_text}");
    let relock = Command::new(rnx())
        .arg("lock")
        .current_dir(&app)
        .output()
        .unwrap();
    assert!(relock.status.success(), "{}", combined(&relock));
    let restored = std::fs::read_to_string(&lock_path).unwrap();
    assert!(restored.contains("sys:exec:sh"), "{restored}");
    let allowed_again = Command::new(rnx())
        .arg("run")
        .arg("--locked")
        .arg("--backend")
        .arg("interpreter")
        .arg("--no-color")
        .arg("-q")
        .current_dir(&app)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(allowed_again.status.code(), Some(0), "{}", combined(&allowed_again));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn open_fds_do_not_leak_into_child() {
    if std::env::consts::OS != "linux" {
        return;
    }
    let dir = fresh_dir("fds");
    let main = dir.join("main.rnx");
    write(
        &main,
        &format!(
            "{FILE_PRELUDE}{PROC_PRELUDE}fn Main(): Int {{\n    let f = File.open(\"marker_fd_test.txt\", OpenMode.Write);\n    f.writeText(\"x\");\n    let out = Process.run(\"ls\", [\"-l\", \"/proc/self/fd\"]);\n    print(out.stdoutText());\n    f.close();\n    return 0;\n}}\n"
        ),
    );
    let out = Command::new(rnx())
        .arg("run")
        .arg(&main)
        .arg("--backend")
        .arg("interpreter")
        .arg("--no-color")
        .arg("-q")
        .stdin(Stdio::null())
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", combined(&out));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("marker_fd_test"),
        "fd leaked into child: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
