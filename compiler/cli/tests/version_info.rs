#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn run(args: &[&str], dir: &std::path::Path) -> (i32, String, String) {
    let out = std::process::Command::new(rnx())
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

fn hardcoded_rnx_version(text: &str) -> Option<&str> {
    let mut rest = text;
    while let Some(pos) = rest.find("rnx ") {
        let after = &rest[pos + 4..];
        let end = after
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(after.len());
        let candidate = &after[..end];
        let parts: Vec<&str> = candidate.split('.').collect();
        if parts.len() == 3
            && parts
                .iter()
                .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
        {
            return Some(&rest[pos..pos + 4 + end]);
        }
        rest = &after[end..];
    }
    None
}

fn markdown_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            markdown_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "md") {
            out.push(path);
        }
    }
}

#[test]
fn test_short_version_is_single_line() {
    let dir = crate::scratch::dir("version-short");
    let (code, stdout, _) = run(&["-V"], &dir);
    assert_eq!(code, 0, "{stdout}");
    assert_eq!(stdout, format!("rnx {}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn test_long_version_has_build_record() {
    let dir = crate::scratch::dir("version-long");
    let (code, stdout, _) = run(&["--version"], &dir);
    assert_eq!(code, 0, "{stdout}");
    let mut lines = stdout.lines();
    assert_eq!(
        lines.next().unwrap(),
        format!("rnx {}", env!("CARGO_PKG_VERSION")),
        "{stdout}"
    );
    for key in ["commit", "built", "install", "binary", "stdlib", "target", "llvm"] {
        assert!(
            stdout.lines().any(|l| l.trim_start().starts_with(&format!("{key} "))),
            "missing {key} in:\n{stdout}"
        );
    }
    let install = stdout
        .lines()
        .find_map(|l| l.trim_start().strip_prefix("install "))
        .map(str::trim)
        .expect("install line");
    assert!(
        install == "standalone" || install.starts_with("managed:"),
        "{stdout}"
    );
    let stdlib = stdout
        .lines()
        .find_map(|l| l.trim_start().strip_prefix("stdlib "))
        .map(str::trim)
        .expect("stdlib line");
    assert!(!stdlib.is_empty(), "{stdout}");
}

#[test]
fn docs_do_not_pin_the_cli_version() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("website")
        .join("src")
        .join("content");
    let mut files = Vec::new();
    markdown_files(&root, &mut files);
    let mut hits = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        if let Some(hit) = hardcoded_rnx_version(&text) {
            hits.push(format!("{}: {hit}", file.display()));
        }
    }
    assert!(
        hits.is_empty(),
        "docs hardcode the rnx version; write `rnx <version>` instead:\n{}",
        hits.join("\n")
    );
}
