use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn fresh_pkg(tag: &str, name: &str, version: &str, lib_src: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-secaudit-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Project.config"),
        format!("export default {{\n    project: {{\n        name: \"{name}\",\n        version: \"{version}\"\n    }},\n    entries: {{ main: \"src/lib.rnx\" }}\n}}\n"),
    )
    .unwrap();
    std::fs::write(dir.join("src/lib.rnx"), lib_src).unwrap();
    dir
}

fn audit(args: &[&str]) -> std::process::Output {
    Command::new(rnx())
        .args(args)
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

const DELEGATED_LIB: &str = "fn loadData(path: String): Int {\n    File.open(path, FileMode.Read);\n    return 0;\n}\n";
const PURE_LIB: &str = "fn add(a: Int, b: Int): Int {\n    return a + b;\n}\n";

#[test]
fn audit_terminal_delegated() {
    let dir = fresh_pkg("term", "delib", "0.3.0", DELEGATED_LIB);
    let out = audit(&["audit", "--path", dir.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{}", combined(&out));
    let text = combined(&out);
    assert!(text.contains("[TIER 2: DELEGATED]"), "{text}");
    assert!(text.contains("fs:delegated"), "{text}");
    assert!(text.contains("File.open"), "{text}");
    assert!(text.contains("loadData"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn audit_json_flag() {
    let dir = fresh_pkg("json", "delib", "0.3.0", DELEGATED_LIB);
    let out = audit(&["audit", "--path", dir.to_str().unwrap(), "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", combined(&out));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    assert_eq!(v["tier"], "delegated");
    assert_eq!(v["capabilities"], serde_json::json!(["fs:delegated"]));
    assert_eq!(v["has_hazard"], false);
    let traces = v["traces"].as_array().expect("traces");
    assert_eq!(traces.len(), 1);
    assert_eq!(traces[0]["capability"], "fs:delegated");
    assert_eq!(traces[0]["is_delegated"], true);
    let chain = traces[0]["nodes"].as_array().expect("nodes");
    assert!(chain.len() >= 2);
    assert_eq!(chain[0]["symbol"], "loadData");
    assert_eq!(chain.last().unwrap()["symbol"], "File.open");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn audit_pure_package() {
    let dir = fresh_pkg("pure", "purelib", "0.1.0", PURE_LIB);
    let out = audit(&["audit", "--path", dir.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{}", combined(&out));
    let text = combined(&out);
    assert!(text.contains("[TIER 1: PURE]"), "{text}");
    assert!(text.contains("100% verified pure computation"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn audit_export_manifest() {
    let dir = fresh_pkg("manifest", "delib", "0.3.0", DELEGATED_LIB);
    let out = audit(&["audit", "--path", dir.to_str().unwrap(), "--export-manifest"]);
    assert_eq!(out.status.code(), Some(0), "{}", combined(&out));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    assert_eq!(v["name"], "delib");
    assert_eq!(v["version"], "0.3.0");
    assert_eq!(v["tier"], "delegated");
    assert_eq!(v["capabilities"], serde_json::json!(["fs:delegated"]));
    assert!(v.get("traces").is_none(), "manifest stays minimal");
    let _ = std::fs::remove_dir_all(&dir);
}

struct McpSession {
    child: std::process::Child,
    next_id: i64,
    reader: BufReader<std::process::ChildStdout>,
}

impl McpSession {
    fn start() -> Self {
        let mut child = Command::new(rnx())
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn rnx mcp");
        let stdout = child.stdout.take().expect("stdout");
        McpSession {
            child,
            next_id: 1,
            reader: BufReader::new(stdout),
        }
    }

    fn request(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        let id = self.next_id;
        self.next_id += 1;
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let stdin = self.child.stdin.as_mut().expect("stdin");
        writeln!(stdin, "{}", msg).expect("write");
        stdin.flush().expect("flush");
        let mut line = String::new();
        self.reader.read_line(&mut line).expect("read");
        serde_json::from_str(&line).expect("valid json response")
    }
}

impl Drop for McpSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

#[test]
fn mcp_inspect_package_capabilities() {
    let dir = fresh_pkg("mcp", "delib", "0.3.0", DELEGATED_LIB);
    let mut s = McpSession::start();
    let init = s.request(
        "initialize",
        serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "rnx-test", "version": "0.0.0" },
        }),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "rnx-mcp");
    s.request("tools/list", serde_json::json!({}));
    let resp = s.request(
        "tools/call",
        serde_json::json!({
            "name": "inspect_package_capabilities",
            "arguments": { "path": dir.to_str().unwrap() },
        }),
    );
    let texts: Vec<String> = resp["result"]["content"]
        .as_array()
        .expect("content array")
        .iter()
        .filter_map(|c| c["text"].as_str().map(|t| t.to_string()))
        .collect();
    let payload: serde_json::Value =
        serde_json::from_str(&texts.join("\n")).expect("tool returns json");
    assert_eq!(payload["tier"], "delegated");
    assert_eq!(payload["capabilities"], serde_json::json!(["fs:delegated"]));
    assert_eq!(payload["has_hazard"], false);
    assert!(!payload["summary"].as_str().unwrap_or("").is_empty());
    let traces = payload["traces"].as_array().expect("traces");
    assert_eq!(traces.len(), 1);
    assert_eq!(traces[0]["nodes"][0]["symbol"], "loadData");
    let _ = std::fs::remove_dir_all(&dir);
}
