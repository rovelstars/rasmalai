use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

struct Session {
    child: std::process::Child,
    next_id: i64,
    reader: BufReader<std::process::ChildStdout>,
}

impl Session {
    fn start(args: &[&str]) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rnx"))
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn rnx");
        let stdout = child.stdout.take().expect("stdout");
        Session { child, next_id: 1, reader: BufReader::new(stdout) }
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
        writeln!(stdin, "{msg}").expect("write");
        stdin.flush().expect("flush");
        let mut line = String::new();
        self.reader.read_line(&mut line).expect("read");
        serde_json::from_str(&line).expect("valid json response")
    }

    fn notify(&mut self, method: &str) {
        let msg = serde_json::json!({ "jsonrpc": "2.0", "method": method });
        let stdin = self.child.stdin.as_mut().expect("stdin");
        writeln!(stdin, "{msg}").expect("write");
        stdin.flush().expect("flush");
    }

    fn call(&mut self, name: &str, args: serde_json::Value) -> serde_json::Value {
        self.request("tools/call", serde_json::json!({ "name": name, "arguments": args }))
    }

    fn content_text(resp: &serde_json::Value) -> String {
        resp["result"]["content"]
            .as_array()
            .expect("content array")
            .iter()
            .filter_map(|c| c["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn is_error(resp: &serde_json::Value) -> bool {
        resp["result"]["isError"] == true
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn handshake(s: &mut Session) {
    let resp = s.request(
        "initialize",
        serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "mcp-test", "version": "0.0.0" },
        }),
    );
    assert_eq!(resp["result"]["protocolVersion"], "2024-11-05", "{resp:?}");
    s.notify("notifications/initialized");
}

#[test]
fn mcp_handshake_and_tools() {
    let mut s = Session::start(&["mcp"]);
    handshake(&mut s);
    let resp = s.request("tools/list", serde_json::json!({}));
    let names: Vec<String> = resp["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|t| t["name"].as_str().expect("name").to_string())
        .collect();
    for want in ["eval_code", "hot_reload", "get_diagnostics"] {
        assert!(names.contains(&want.to_string()), "missing tool {want}: {names:?}");
    }
}

#[test]
fn mcp_eval_keeps_state() {
    let mut s = Session::start(&["mcp"]);
    handshake(&mut s);
    let def = s.call("eval_code", serde_json::json!({ "code": "let count = 42;" }));
    assert!(!Session::is_error(&def), "{def:?}");
    let val = s.call("eval_code", serde_json::json!({ "code": "count * 2" }));
    assert!(!Session::is_error(&val), "{val:?}");
    assert!(Session::content_text(&val).contains("84"), "{val:?}");
}

#[test]
fn mcp_eval_error_keeps_session_alive() {
    let mut s = Session::start(&["mcp"]);
    handshake(&mut s);
    let bad = s.call("eval_code", serde_json::json!({ "code": "let bad = {" }));
    assert!(Session::is_error(&bad), "{bad:?}");
    let text = Session::content_text(&bad);
    assert!(!text.is_empty(), "{bad:?}");
    let after = s.call("eval_code", serde_json::json!({ "code": "7 + 8" }));
    assert!(!Session::is_error(&after), "{after:?}");
    assert!(Session::content_text(&after).contains("15"), "{after:?}");
}

#[test]
fn mcp_hot_reload_without_dev_reports_error() {
    let mut s = Session::start(&["mcp"]);
    handshake(&mut s);
    let resp = s.call("hot_reload", serde_json::json!({}));
    let text = Session::content_text(&resp);
    let report: serde_json::Value = serde_json::from_str(&text).expect("hot reload returns JSON");
    assert_eq!(report["status"], "error", "{report:?}");
    assert!(report["diagnostics"].as_array().is_some(), "{report:?}");
}

#[test]
fn mcp_dev_mcp_serves_reload() {
    let dir = std::env::temp_dir().join(format!("rnx-mcp-dev-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("tempdir");
    let main = dir.join("main.rnx");
    std::fs::write(&main, "fn Main(): Int {\n    return 1;\n}\n").expect("write");
    let mut s = Session::start(&["dev", main.to_str().expect("path"), "--mcp"]);
    handshake(&mut s);
    let resp = s.call("hot_reload", serde_json::json!({}));
    let text = Session::content_text(&resp);
    let report: serde_json::Value = serde_json::from_str(&text).expect("hot reload returns JSON");
    assert_eq!(report["status"], "no_change", "{report:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mcp_get_diagnostics_on_file() {
    let dir = std::env::temp_dir().join(format!("rnx-mcp-diag-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("tempdir");
    let bad = dir.join("bad.rnx");
    std::fs::write(&bad, "fn Main(): Int {\n    return nope;\n}\n").expect("write");
    let good = dir.join("good.rnx");
    std::fs::write(&good, "fn Main(): Int {\n    return 1;\n}\n").expect("write");
    let mut s = Session::start(&["mcp"]);
    handshake(&mut s);
    let resp = s.call("get_diagnostics", serde_json::json!({ "path": bad.to_string_lossy() }));
    assert!(Session::is_error(&resp), "{resp:?}");
    let out: serde_json::Value =
        serde_json::from_str(&Session::content_text(&resp)).expect("diagnostics JSON");
    assert!(!out["errors"].as_array().expect("errors").is_empty(), "{out:?}");
    let ok = s.call("get_diagnostics", serde_json::json!({ "path": good.to_string_lossy() }));
    assert!(!Session::is_error(&ok), "{ok:?}");
    let out: serde_json::Value =
        serde_json::from_str(&Session::content_text(&ok)).expect("diagnostics JSON");
    assert!(out["errors"].as_array().expect("errors").is_empty(), "{out:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
