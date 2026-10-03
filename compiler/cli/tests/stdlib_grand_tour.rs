use std::path::PathBuf;

const EXPECTED: &str = "Phase 5 Grand Tour verified: 42\n";

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/grand_tour/src/main.rnx")
}

fn tour_tmp_gone() {
    assert!(
        !std::path::Path::new("target/grand_tour_tmp.txt").exists(),
        "grand tour must remove its temp file"
    );
    let _ = std::fs::remove_dir("target");
}

#[test]
fn grand_tour_interpreter_and_native_binary() {
    std::fs::create_dir_all("target")
        .unwrap_or_else(|e| panic!("create target dir for grand tour io: {e}"));
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let run = std::process::Command::new(rnx)
        .arg("run")
        .arg(fixture())
        .output()
        .unwrap();
    assert_eq!(run.status.code().unwrap(), 42, "{}", String::from_utf8_lossy(&run.stderr));
    let stdout = String::from_utf8(run.stdout).unwrap();
    assert!(stdout.contains(EXPECTED.trim()), "{stdout}");
    tour_tmp_gone();

    let dir = std::env::temp_dir().join(format!("rnx-grand-tour-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let out_path = dir.join("grand_tour");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(fixture())
        .arg("-o")
        .arg(&out_path)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let meta = std::fs::metadata(&out_path).unwrap();
    assert!(meta.len() < 1_000_000, "dev binary links the runtime dynamically, not the archive");
    std::fs::create_dir_all("target")
        .unwrap_or_else(|e| panic!("recreate target dir for grand tour io: {e}"));
    let run = std::process::Command::new(&out_path).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 42);
    assert_eq!(String::from_utf8(run.stdout).unwrap(), EXPECTED);
    let _ = std::fs::remove_dir_all(&dir);
    tour_tmp_gone();
}
