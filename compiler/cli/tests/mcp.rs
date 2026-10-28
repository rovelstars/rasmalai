use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

// End-to-end pin for `rnx mcp`: speaks JSON-RPC over stdio like a real
// harness (initialize, tools/list, tools/call) and asserts each tool.

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

struct Session {
    child: std::process::Child,
    next_id: i64,
    reader: BufReader<std::process::ChildStdout>,
}

impl Session {
    fn start() -> Self {
        let mut child = Command::new(rnx())
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn rnx mcp");
        let stdout = child.stdout.take().expect("stdout");
        Session {
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

    fn notify(&mut self, method: &str) {
        let msg = serde_json::json!({ "jsonrpc": "2.0", "method": method });
        let stdin = self.child.stdin.as_mut().expect("stdin");
        writeln!(stdin, "{}", msg).expect("write");
        stdin.flush().expect("flush");
    }

    fn call(&mut self, name: &str, args: serde_json::Value) -> serde_json::Value {
        self.request(
            "tools/call",
            serde_json::json!({ "name": name, "arguments": args }),
        )
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
            "clientInfo": { "name": "rnx-test", "version": "0.0.0" },
        }),
    );
    assert_eq!(resp["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(resp["result"]["serverInfo"]["name"], "rnx-mcp");
    let instructions = resp["result"]["instructions"].as_str().unwrap_or("");
    assert!(instructions.contains("Unified 64-bit Numerics"), "{instructions:?}");
    assert!(instructions.contains("NO `mut` keyword"), "{instructions:?}");
    assert!(instructions.contains("`new Meter(20)`"), "{instructions:?}");
    assert!(instructions.contains("E204"), "{instructions:?}");
    let caps = &resp["result"]["capabilities"];
    assert!(caps.get("tools").is_some(), "{caps:?}");
    assert!(caps.get("resources").is_some(), "{caps:?}");
    assert!(caps.get("prompts").is_some(), "{caps:?}");
    s.notify("notifications/initialized");
}

#[test]
fn mcp_lists_expected_tools() {
    let mut s = Session::start();
    handshake(&mut s);
    let resp = s.request("tools/list", serde_json::json!({}));
    let mut names: Vec<String> = resp["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|t| t["name"].as_str().expect("name").to_string())
        .collect();
    names.sort();
    let mut expected: Vec<String> = [
        "check", "run", "fmt", "explain", "version", "rasmalai_lookup_symbol",
        "inspect_package_capabilities", "eval_code", "hot_reload", "get_diagnostics",
        "build", "test", "lint", "doc", "doctor", "cache_status", "package_resolve",
    ]
    .iter()
    .map(|n| n.to_string())
    .collect();
    expected.sort();
    assert_eq!(names, expected, "tools/list must match the documented set");
}

#[test]
fn mcp_check_ok_and_error() {
    let mut s = Session::start();
    handshake(&mut s);
    let ok = s.call(
        "check",
        serde_json::json!({ "source": "fn Main(): Int {\n    return 40 + 2;\n}\n" }),
    );
    let ok_text = Session::content_text(&ok);
    let ok_json: serde_json::Value = serde_json::from_str(&ok_text).expect("check returns JSON");
    assert_eq!(ok_json["status"], "ok", "{ok_text:?}");
    assert!(ok_json["errors"].as_array().expect("errors").is_empty(), "{ok_text:?}");
    let bad = s.call(
        "check",
        serde_json::json!({ "source": "fn Main(): Int {\n    return nope;\n}\n" }),
    );
    let text = Session::content_text(&bad);
    let bad_json: serde_json::Value = serde_json::from_str(&text).expect("check returns JSON");
    assert_eq!(bad_json["status"], "error", "{text:?}");
    let errors = bad_json["errors"].as_array().expect("errors");
    assert!(!errors.is_empty(), "{text:?}");
    assert!(errors.iter().any(|e| e["code"] == "E303"), "{text:?}");
    assert!(errors.iter().all(|e| e["line"].as_u64().unwrap_or(0) > 0), "{text:?}");
    assert!(errors.iter().all(|e| !e["excerpt"].as_str().unwrap_or("").is_empty()), "{text:?}");
    let pretty = s.call(
        "check",
        serde_json::json!({ "source": "fn Main(): Int {\n    return nope;\n}\n", "pretty": true }),
    );
    assert!(Session::content_text(&pretty).contains("E303"), "{pretty:?}");
}

#[test]
fn mcp_run_returns_value() {
    let mut s = Session::start();
    handshake(&mut s);
    let resp = s.call(
        "run",
        serde_json::json!({ "source": "fn Main(): Int {\n    print(40 + 2);\n    return 0;\n}\n" }),
    );
    let text = Session::content_text(&resp);
    let out: serde_json::Value = serde_json::from_str(&text).expect("run returns JSON");
    assert_eq!(out["status"], "value", "{text:?}");
    assert_eq!(out["value"], "0", "{text:?}");
    let lines = out["stdout_lines"].as_array().expect("stdout_lines");
    assert!(lines.iter().any(|l| l.as_str().unwrap_or("").contains("42")), "{text:?}");
    assert_eq!(out["truncated_stdout"], false, "{text:?}");
    let pretty = s.call(
        "run",
        serde_json::json!({ "source": "fn Main(): Int {\n    print(40 + 2);\n    return 0;\n}\n", "pretty": true }),
    );
    let pretty_text = Session::content_text(&pretty);
    assert!(pretty_text.contains("42"), "{pretty_text:?}");
    assert!(pretty_text.contains("=> 0"), "{pretty_text:?}");
    let capped = s.call(
        "run",
        serde_json::json!({ "source": "fn Main(): Int {\n    print(40 + 2);\n    return 0;\n}\n", "max_output_bytes": 2 }),
    );
    let capped_text = Session::content_text(&capped);
    let capped_json: serde_json::Value = serde_json::from_str(&capped_text).expect("run returns JSON");
    assert_eq!(capped_json["truncated_stdout"], true, "{capped_text:?}");
}

#[test]
fn mcp_fmt_explain_version() {
    let mut s = Session::start();
    handshake(&mut s);
    let fmt = s.call(
        "fmt",
        serde_json::json!({ "source": "fn Main(): Int {\n    return 1;\n}\n" }),
    );
    assert!(Session::content_text(&fmt).contains("already formatted"), "{fmt:?}");
    let exp = s.call("explain", serde_json::json!({ "code": "E108" }));
    assert!(Session::content_text(&exp).contains("E108"), "{exp:?}");
    let ver = s.call("version", serde_json::json!({}));
    assert!(Session::content_text(&ver).contains("rnx "), "{ver:?}");
    let unknown = s.call("explain", serde_json::json!({ "code": "E999" }));
    assert!(unknown["result"]["isError"] == true, "{unknown:?}");
}

#[test]
fn mcp_resources_list_and_read() {
    let mut s = Session::start();
    handshake(&mut s);
    let resp = s.request("resources/list", serde_json::json!({}));
    let uris: Vec<String> = resp["result"]["resources"]
        .as_array()
        .expect("resources array")
        .iter()
        .map(|r| r["uri"].as_str().expect("uri").to_string())
        .collect();
    for want in [
        "rasmalai://spec/grammar",
        "rasmalai://spec/architecture",
        "rasmalai://spec/manifest",
        "rasmalai://stdlib/api.json",
    ] {
        assert!(uris.contains(&want.to_string()), "missing resource {want}: {uris:?}");
    }
    let read = s.request(
        "resources/read",
        serde_json::json!({ "uri": "rasmalai://spec/grammar" }),
    );
    let text = read["result"]["contents"]
        .as_array()
        .expect("contents array")
        .iter()
        .filter_map(|c| c["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("`${name}`") || text.contains("interpolation"), "{text:?}");
    assert!(text.contains("E108"), "{text:?}");
    let arch = s.request(
        "resources/read",
        serde_json::json!({ "uri": "rasmalai://spec/architecture" }),
    );
    let arch_text = arch["result"]["contents"][0]["text"].as_str().unwrap_or("");
    assert!(arch_text.contains("GenRef"), "{arch_text:?}");
    let manifest = s.request(
        "resources/read",
        serde_json::json!({ "uri": "rasmalai://spec/manifest" }),
    );
    let manifest_text = manifest["result"]["contents"][0]["text"].as_str().unwrap_or("");
    assert!(manifest_text.contains("export default"), "{manifest_text:?}");
    assert!(manifest_text.contains("dependencies"), "{manifest_text:?}");
    let api = s.request(
        "resources/read",
        serde_json::json!({ "uri": "rasmalai://stdlib/api.json" }),
    );
    let api_text = api["result"]["contents"][0]["text"].as_str().unwrap_or("");
    let parsed: serde_json::Value = serde_json::from_str(api_text).expect("api.json parses");
    assert!(parsed.get("modules").is_some(), "{api_text:?}");
    let missing = s.request(
        "resources/read",
        serde_json::json!({ "uri": "rasmalai://spec/nope" }),
    );
    assert!(missing.get("error").is_some(), "{missing:?}");
}

#[test]
fn mcp_prompts_list_and_get() {
    let mut s = Session::start();
    handshake(&mut s);
    let resp = s.request("prompts/list", serde_json::json!({}));
    let names: Vec<String> = resp["result"]["prompts"]
        .as_array()
        .expect("prompts array")
        .iter()
        .map(|p| p["name"].as_str().expect("name").to_string())
        .collect();
    for want in ["rasmalai-expert", "convert-to-rasmalai"] {
        assert!(names.contains(&want.to_string()), "missing prompt {want}: {names:?}");
    }
    let expert = s.request(
        "prompts/get",
        serde_json::json!({ "name": "rasmalai-expert" }),
    );
    let expert_text = expert["result"]["messages"][0]["content"]["text"]
        .as_str()
        .unwrap_or("");
    assert!(expert_text.contains("rasmalai://spec/grammar"), "{expert_text:?}");
    assert!(expert_text.contains("`check`"), "{expert_text:?}");
    let convert = s.request(
        "prompts/get",
        serde_json::json!({
            "name": "convert-to-rasmalai",
            "arguments": { "source_language": "go", "code": "x := 1" },
        }),
    );
    let convert_text = convert["result"]["messages"][0]["content"]["text"]
        .as_str()
        .unwrap_or("");
    assert!(convert_text.contains("go"), "{convert_text:?}");
    assert!(convert_text.contains("@std/sync"), "{convert_text:?}");
    let missing_arg = s.request(
        "prompts/get",
        serde_json::json!({ "name": "convert-to-rasmalai", "arguments": {} }),
    );
    assert!(missing_arg.get("error").is_some(), "{missing_arg:?}");
    let unknown = s.request("prompts/get", serde_json::json!({ "name": "nope" }));
    assert!(unknown.get("error").is_some(), "{unknown:?}");
}

#[test]
fn mcp_lookup_symbol() {
    let mut s = Session::start();
    handshake(&mut s);
    let resp = s.call(
        "rasmalai_lookup_symbol",
        serde_json::json!({ "query": "ByteBuffer" }),
    );
    let text = Session::content_text(&resp);
    assert!(text.contains("ByteBuffer"), "{text:?}");
    assert!(text.contains("bytes"), "{text:?}");
    assert!(text.contains("fn allocate"), "{text:?}");
    let scoped = s.call(
        "rasmalai_lookup_symbol",
        serde_json::json!({ "query": "read", "module": "@std/bytes" }),
    );
    let scoped_text = Session::content_text(&scoped);
    assert!(scoped_text.contains("@std/bytes") || scoped_text.contains("std/bytes"), "{scoped_text:?}");
    let miss = s.call(
        "rasmalai_lookup_symbol",
        serde_json::json!({ "query": "NoSuchSymbolZZZ" }),
    );
    assert!(Session::content_text(&miss).contains("no symbols matching"), "{miss:?}");
}
