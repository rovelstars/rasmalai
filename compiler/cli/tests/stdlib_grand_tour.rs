#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::path::PathBuf;

const EXPECTED: &str = "Phase 5 Grand Tour verified: 42\n";

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/grand_tour/src/main.rnx")
}

fn tour_tmp_gone(dir: &std::path::Path) {
    assert!(
        !dir.join("target/grand_tour_tmp.txt").exists(),
        "grand tour must remove its temp file"
    );
}

#[test]
fn grand_tour_interpreter_and_native_binary() {
    // The grand tour fixture writes `target/grand_tour_tmp.txt` relative to
    // its working directory. Running it with the package directory as cwd
    // would share that path with every other grand tour run, so give this
    // test a unique scratch cwd instead.
    let dir = crate::scratch::dir("grand-tour");
    std::fs::create_dir_all(dir.join("target"))
        .unwrap_or_else(|e| panic!("create target dir for grand tour io: {e}"));
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let run = std::process::Command::new(rnx)
        .arg("run")
        .arg(fixture())
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(run.status.code().unwrap(), 42, "{}", String::from_utf8_lossy(&run.stderr));
    let stdout = String::from_utf8(run.stdout).unwrap();
    assert!(stdout.contains(EXPECTED.trim()), "{stdout}");
    tour_tmp_gone(&dir);

    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(fixture())
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = String::from_utf8_lossy(&build.stdout).lines().rev().find_map(|l| l.strip_prefix("artifact: ")).map(|s| s.trim().trim_end_matches(" (fresh)").to_string()).expect("build prints artifact path");
    // Fresh CI builds may not have the dev shared runtime next to the
    // compiler, in which case the dev build statically links the archive
    // by design. Assert the dynamic link only when the shared runtime is
    // actually available.
    let shared = std::path::Path::new(rnx)
        .parent()
        .map(|d| d.join("libruntime_native.so").is_file())
        .unwrap_or(false);
    if shared {
        let deps = std::process::Command::new("readelf")
            .arg("-d")
            .arg(&bin)
            .output()
            .expect("readelf");
        let text = String::from_utf8_lossy(&deps.stdout);
        assert!(
            text.contains("libruntime_native.so"),
            "dev binary does not depend on the shared runtime: {text}"
        );
    }
    let run = std::process::Command::new(&bin).current_dir(&dir).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 42);
    assert_eq!(String::from_utf8(run.stdout).unwrap(), EXPECTED);
    tour_tmp_gone(&dir);
}
