#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

fn fresh_script_dir(tag: &str) -> crate::scratch::Scratch {
    crate::scratch::dir(&format!("stale-{tag}"))
}

fn build(main: &std::path::Path) -> (bool, String) {
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let out = std::process::Command::new(rnx)
        .arg("build")
        .arg(main)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let line = String::from_utf8_lossy(&out.stdout)
        .lines()
        .rev()
        .find(|l| l.starts_with("artifact: "))
        .unwrap_or_else(|| panic!("build prints artifact path"))
        .to_string();
    let fresh = line.ends_with(" (fresh)");
    let bin = line
        .strip_prefix("artifact: ")
        .unwrap()
        .trim_end_matches(" (fresh)")
        .to_string();
    (fresh, bin)
}

fn run_bin(bin: &str) -> (i32, String) {
    let run = std::process::Command::new(bin).output().unwrap();
    (run.status.code().unwrap(), String::from_utf8(run.stdout).unwrap())
}

const MAIN_SRC: &str = "import { helper } from \"./m2\";\nfn Main(): Int {\n    print(helper());\n    return helper();\n}\n";

#[test]
fn edited_relative_import_invalidates_build() {
    let dir = fresh_script_dir("sibling");
    let main = dir.join("main.rnx");
    std::fs::write(&main, MAIN_SRC).unwrap();
    std::fs::write(dir.join("m2.rnx"), "export fn helper(): Int {\n    return 1;\n}\n")
        .unwrap();

    let (fresh, bin) = build(&main);
    assert!(!fresh, "cold build must not report fresh");
    assert_eq!(run_bin(&bin), (1, "1\n".to_string()));

    std::fs::write(dir.join("m2.rnx"), "export fn helper(): Int {\n    return 2;\n}\n")
        .unwrap();
    let (fresh, bin) = build(&main);
    assert!(!fresh, "editing the imported file must invalidate the build");
    assert_eq!(run_bin(&bin), (2, "2\n".to_string()));

    let (fresh, bin) = build(&main);
    assert!(fresh, "no-op rebuild must stay fresh");
    assert_eq!(run_bin(&bin), (2, "2\n".to_string()));
}

#[test]
fn edited_entry_still_invalidates_build() {
    let dir = fresh_script_dir("entry");
    let main = dir.join("main.rnx");
    std::fs::write(&main, MAIN_SRC).unwrap();
    std::fs::write(dir.join("m2.rnx"), "export fn helper(): Int {\n    return 1;\n}\n")
        .unwrap();

    let (_, bin) = build(&main);
    assert_eq!(run_bin(&bin), (1, "1\n".to_string()));

    std::fs::write(
        &main,
        "import { helper } from \"./m2\";\nfn Main(): Int {\n    print(helper() + 10);\n    return helper() + 10;\n}\n",
    )
    .unwrap();
    let (fresh, bin) = build(&main);
    assert!(!fresh, "editing the entry file must invalidate the build");
    assert_eq!(run_bin(&bin), (11, "11\n".to_string()));
}

#[test]
fn edited_nested_import_outside_src_invalidates_build() {
    let dir = fresh_script_dir("libdepth");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::write(
        dir.join("Project.config"),
        "export default {\n    project: {\n        name: \"staleapp\",\n        version: \"0.1.0\"\n    }}\n",
    )
    .unwrap();
    let main = dir.join("src").join("main.rnx");
    std::fs::write(&main, MAIN_SRC).unwrap();
    std::fs::write(
        dir.join("lib").join("m2.rnx"),
        "export fn helper(): Int {\n    return 1;\n}\n",
    )
    .unwrap();
    // Import resolves relative to the importer: src/main.rnx -> ../lib/m2.
    std::fs::write(
        &main,
        "import { helper } from \"../lib/m2\";\nfn Main(): Int {\n    print(helper());\n    return helper();\n}\n",
    )
    .unwrap();

    let (fresh, bin) = build(&main);
    assert!(!fresh, "cold build must not report fresh");
    assert_eq!(run_bin(&bin), (1, "1\n".to_string()));

    std::fs::write(
        dir.join("lib").join("m2.rnx"),
        "export fn helper(): Int {\n    return 2;\n}\n",
    )
    .unwrap();
    let (fresh, bin) = build(&main);
    assert!(!fresh, "editing an imported file outside src/ must invalidate");
    assert_eq!(run_bin(&bin), (2, "2\n".to_string()));
}
