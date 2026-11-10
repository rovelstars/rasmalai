#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;
#[path = "../../testkit/pack.rs"]
mod pack_fixture;

fn rnx() -> &'static str {
    env!("CARGO_BIN_EXE_rnx")
}

fn run(args: &[&str]) -> std::process::Output {
    let cache = crate::scratch::dir("target-list-cache");
    run_with_cache(args, &cache)
}

fn run_with_cache(args: &[&str], cache: &std::path::Path) -> std::process::Output {
    std::process::Command::new(rnx())
        .args(args)
        .env("RNX_CACHE_HOME", cache)
        .env_remove("RNX_SYSROOT")
        .output()
        .unwrap()
}

fn triples(text: &str) -> Vec<String> {
    text.lines()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.split_whitespace().next().unwrap_or("").to_string())
        .collect()
}

#[test]
fn target_list_is_sorted_and_stable() {
    let first = run(&["target", "list"]);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let text = String::from_utf8_lossy(&first.stdout).into_owned();
    let header = text.lines().next().unwrap_or("");
    for column in ["TRIPLE", "TIER", "OBJECT", "CAPABILITIES", "PACK"] {
        assert!(header.contains(column), "missing {column} in {header:?}");
    }
    let names = triples(&text);
    assert!(!names.is_empty());
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
    let second = run(&["target", "list"]);
    assert_eq!(first.stdout, second.stdout);
}

#[test]
fn target_list_json_parses_and_is_sorted() {
    let output = run(&["target", "list", "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("target list --json is valid JSON");
    let rows = value.as_array().expect("JSON array");
    assert_eq!(rows.len(), 9);
    let names: Vec<String> = rows
        .iter()
        .map(|row| {
            assert!(row.get("triple").and_then(|v| v.as_str()).is_some());
            assert!(row.get("tier").and_then(|v| v.as_str()).is_some());
            assert!(row.get("object").and_then(|v| v.as_str()).is_some());
            assert!(row.get("capabilities").and_then(|v| v.as_array()).is_some());
            assert!(row.get("pack").and_then(|v| v.as_str()).is_some());
            row["triple"].as_str().unwrap_or("").to_string()
        })
        .collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
}

#[test]
fn target_list_reports_installed_packs() {
    let cache = crate::scratch::dir("target-list-pack");
    let fixture = crate::scratch::dir("target-list-pack-fixture");
    crate::pack_fixture::write_pack(&fixture, "x86_64-unknown-linux-musl", "list");
    let add = run_with_cache(
        &["sysroot", "add", "--local", fixture.to_str().unwrap()],
        &cache,
    );
    assert!(add.status.success(), "{}", String::from_utf8_lossy(&add.stderr));
    let output = run_with_cache(&["target", "list"], &cache);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let musl = text
        .lines()
        .find(|line| line.starts_with("x86_64-unknown-linux-musl"))
        .expect("musl row");
    assert!(musl.contains("installed sha256:"), "{musl}");
    let host = text
        .lines()
        .find(|line| line.starts_with("x86_64-unknown-linux-gnu"))
        .expect("host row");
    assert!(host.trim_end().ends_with("missing"), "{host}");

    let json = run_with_cache(&["target", "list", "--json"], &cache);
    let rows: serde_json::Value = serde_json::from_slice(&json.stdout).expect("JSON array");
    let rows = rows.as_array().expect("array");
    let musl = rows
        .iter()
        .find(|row| row["triple"] == "x86_64-unknown-linux-musl")
        .expect("musl row");
    assert!(
        musl["pack"].as_str().unwrap_or("").starts_with("installed sha256:"),
        "{musl}"
    );
}

#[test]
fn target_list_tier_filters() {
    let output = run(&["target", "list", "--tier", "1"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let names = triples(&text);
    assert_eq!(names, vec!["x86_64-unknown-linux-gnu".to_string()]);
    assert!(text.lines().nth(1).unwrap_or("").split_whitespace().nth(1) == Some("1"));
}

#[test]
fn target_list_rejects_an_invalid_tier() {
    let output = run(&["target", "list", "--tier", "4"]);
    assert_eq!(output.status.code(), Some(2));
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(err.contains("tier"), "{err}");
}
