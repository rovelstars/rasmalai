#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

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
        Self::start_with_env(&[])
    }

    fn start_with_env(envs: &[(&str, &str)]) -> Self {
        let mut cmd = Command::new(rnx());
        cmd.arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().expect("spawn rnx mcp");
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

    fn content_json(resp: &serde_json::Value) -> serde_json::Value {
        let text = Self::content_text(resp);
        serde_json::from_str(&text).expect("tool returns JSON")
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
            "clientInfo": { "name": "mcp-tools", "version": "0.0.0" },
        }),
    );
    assert_eq!(resp["result"]["protocolVersion"], "2024-11-05", "{resp:?}");
    s.notify("notifications/initialized");
}

fn write_project(dir: &std::path::Path, name: &str, files: &[(&str, &str)]) {
    std::fs::write(
        dir.join("Project.config"),
        format!("export default {{\n    project: {{\n        name: \"{name}\",\n        version: \"0.1.0\"\n    }}\n}}\n"),
    )
    .unwrap();
    let src = dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    for (file, contents) in files {
        std::fs::write(src.join(file), contents).unwrap();
    }
}

const MATH_LIB: &str = "fn double(x: Int): Int {\n    return x * 2;\n}\n";
const MAIN_OK: &str = "import { double } from \"./math\";\n\nfn Main(): Int {\n    return double(21);\n}\n";

#[test]
fn mcp_check_project_aware_resolves_relative_imports() {
    let dir = crate::scratch::dir("mcp-tools-check");
    write_project(&dir, "checkpkg", &[("main.rnx", MAIN_OK), ("math.rnx", MATH_LIB)]);
    let main = dir.join("src").join("main.rnx");
    let mut s = Session::start();
    handshake(&mut s);
    let resp = s.call("check", serde_json::json!({ "path": main.to_str().unwrap() }));
    let out = Session::content_json(&resp);
    assert_eq!(out["status"], "ok", "{out:?}");
    assert!(out["errors"].as_array().expect("errors").is_empty(), "{out:?}");
    let diag = s.call("get_diagnostics", serde_json::json!({ "path": main.to_str().unwrap() }));
    assert_eq!(diag["result"]["isError"], false, "{diag:?}");
    let diag_json = Session::content_json(&diag);
    assert_eq!(diag_json["status"], "ok", "{diag_json:?}");
}

#[test]
fn mcp_check_snippet_spans_point_at_source_lines() {
    let mut s = Session::start();
    handshake(&mut s);
    let resp = s.call(
        "check",
        serde_json::json!({ "source": "fn Main(): Int {\n    return nope;\n}\n" }),
    );
    let out = Session::content_json(&resp);
    assert_eq!(out["status"], "error", "{out:?}");
    let errors = out["errors"].as_array().expect("errors");
    assert!(!errors.is_empty(), "{out:?}");
    assert!(errors.iter().any(|e| e["code"] == "E303"), "{out:?}");
    for e in errors {
        assert!(e["line"].as_u64().unwrap_or(0) >= 2, "{e:?}");
        assert!(!e["excerpt"].as_str().unwrap_or("").is_empty(), "{e:?}");
    }
}

#[test]
fn mcp_check_missing_path_fails_loudly() {
    let mut s = Session::start();
    handshake(&mut s);
    let resp = s.call("check", serde_json::json!({ "path": "/nonexistent-dir-zzz/nope.rnx" }));
    assert!(Session::is_error(&resp), "{resp:?}");
}

#[test]
fn mcp_run_structured_and_timeout_abandonment() {
    let mut s = Session::start_with_env(&[("RNX_MCP_RUN_TIMEOUT_SECS", "1")]);
    handshake(&mut s);
    let resp = s.call(
        "run",
        serde_json::json!({ "source": "fn Main(): Int {\n    print(40 + 2);\n    return 0;\n}\n" }),
    );
    let out = Session::content_json(&resp);
    assert_eq!(out["status"], "value", "{out:?}");
    assert_eq!(out["value"], "0", "{out:?}");
    let stuck = s.call(
        "run",
        serde_json::json!({ "source": "fn Main(): Int {\n    let i = 0;\n    while i >= 0 {\n        i = i + 1;\n    }\n    return 0;\n}\n" }),
    );
    assert!(Session::is_error(&stuck), "{stuck:?}");
    let message = Session::content_text(&stuck);
    assert!(message.contains("1s limit") || message.contains("abandoned"), "{stuck:?}");
    let refused = s.call("run", serde_json::json!({ "source": "fn Main(): Int {\n    return 1;\n}\n" }));
    assert!(Session::is_error(&refused), "{refused:?}");
    let refused_message = Session::content_text(&refused);
    assert!(refused_message.contains("still executing"), "{refused:?}");
}

#[test]
fn mcp_fmt_check_diff_and_write_modes() {
    let dir = crate::scratch::dir("mcp-tools-fmt");
    let file = dir.join("messy.rnx");
    std::fs::write(&file, "fn Main():Int{\nreturn 1;\n}\n").unwrap();
    let mut s = Session::start();
    handshake(&mut s);
    let check = s.call("fmt", serde_json::json!({ "path": file.to_str().unwrap(), "check": true }));
    assert!(Session::content_text(&check).contains("unformatted"), "{check:?}");
    let diff = s.call("fmt", serde_json::json!({ "path": file.to_str().unwrap(), "diff": true }));
    let diff_text = Session::content_text(&diff);
    assert!(diff_text.contains("fn Main()"), "{diff_text:?}");
    assert!(diff_text.contains('-') && diff_text.contains('+'), "{diff_text:?}");
    let write = s.call("fmt", serde_json::json!({ "path": file.to_str().unwrap() }));
    assert!(write.get("result").is_some(), "{write:?}");
    let clean = s.call("fmt", serde_json::json!({ "path": file.to_str().unwrap(), "check": true }));
    assert!(Session::content_text(&clean).contains("already formatted"), "{clean:?}");
    let snippet = s.call("fmt", serde_json::json!({ "source": "fn Main():Int{\nreturn 1;\n}\n", "check": true }));
    assert!(Session::content_text(&snippet).contains("needs formatting"), "{snippet:?}");
}

#[test]
fn mcp_lint_reports_findings_as_json() {
    let dir = crate::scratch::dir("mcp-tools-lint");
    write_project(
        &dir,
        "lintpkg",
        &[("main.rnx", "fn Main(): Int {\n    let unused_thing = 1;\n    return 0;\n}\n")],
    );
    let main = dir.join("src").join("main.rnx");
    let mut s = Session::start();
    handshake(&mut s);
    let resp = s.call("lint", serde_json::json!({ "path": main.to_str().unwrap() }));
    let out = Session::content_json(&resp);
    assert_eq!(out["status"], "findings", "{out:?}");
    assert!(out["findings_total"].as_u64().unwrap_or(0) > 0, "{out:?}");
    let findings = out["findings"].as_array().expect("findings");
    assert!(findings.iter().any(|f| f["code"] == "L001"), "{out:?}");
    let clean = s.call("lint", serde_json::json!({ "path": main.to_str().unwrap(), "deny_warnings": true }));
    let clean_json = Session::content_json(&clean);
    assert!(clean_json["findings_total"].as_u64().unwrap_or(0) > 0, "{clean_json:?}");
    assert_eq!(clean_json["code"], 1, "{clean_json:?}");
}

#[test]
fn mcp_doc_collects_project_api() {
    let dir = crate::scratch::dir("mcp-tools-doc");
    write_project(&dir, "docpkg", &[("main.rnx", MAIN_OK), ("math.rnx", MATH_LIB)]);
    let mut s = Session::start();
    handshake(&mut s);
    let resp = s.call("doc", serde_json::json!({ "path": dir.to_str().unwrap() }));
    let out = Session::content_json(&resp);
    assert_eq!(out["status"], "ok", "{out:?}");
    assert!(out["module_count"].as_u64().unwrap_or(0) >= 1, "{out:?}");
    assert!(!out["preview"].as_str().unwrap_or("").is_empty(), "{out:?}");
    let missing = s.call("doc", serde_json::json!({ "path": "/nonexistent-dir-zzz" }));
    assert!(Session::is_error(&missing), "{missing:?}");
}

#[test]
fn mcp_doctor_and_cache_status_are_read_only_shapes() {
    let mut s = Session::start();
    handshake(&mut s);
    let doctor = s.call("doctor", serde_json::json!({}));
    let d = Session::content_json(&doctor);
    assert_eq!(d["status"], "ok", "{d:?}");
    assert!(d["stdlib"].is_object(), "{d:?}");
    assert!(d["products"].is_object(), "{d:?}");
    assert!(d["dependencies"].is_array(), "{d:?}");
    let cache = s.call("cache_status", serde_json::json!({}));
    let c = Session::content_json(&cache);
    assert_eq!(c["status"], "ok", "{c:?}");
    assert!(c["global"]["bytes"].as_u64().is_some(), "{c:?}");
    assert!(c["store"]["bytes"].as_u64().is_some(), "{c:?}");
    assert!(c["project"]["bytes"].as_u64().is_some(), "{c:?}");
}

#[test]
fn mcp_package_resolve_reports_cache_state_without_network() {
    let mut s = Session::start_with_env(&[("RNX_REGISTRY", "http://127.0.0.1:9/")]);
    handshake(&mut s);
    let resp = s.call(
        "package_resolve",
        serde_json::json!({ "name": "@scope/definitely-not-a-real-package-zzz" }),
    );
    let out = Session::content_json(&resp);
    assert_eq!(out["status"], "ok", "{out:?}");
    assert_eq!(out["resolved"], serde_json::Value::Null, "{out:?}");
    assert_eq!(out["download_needed"], true, "{out:?}");
    assert!(out["cached_versions"].as_array().expect("versions").is_empty(), "{out:?}");
    assert!(out["hint"].as_str().unwrap_or("").contains("rnx download"), "{out:?}");
    let bad_range = s.call(
        "package_resolve",
        serde_json::json!({ "name": "httpkit", "range": "not a range \\" }),
    );
    assert!(Session::is_error(&bad_range), "{bad_range:?}");
    let empty = s.call("package_resolve", serde_json::json!({ "name": "   " }));
    assert!(Session::is_error(&empty), "{empty:?}");
}

#[test]
fn mcp_audit_parity_licenses_drift_and_offline_advisories() {
    let dir = crate::scratch::dir("mcp-tools-audit");
    std::fs::write(
        dir.join("Project.config"),
        "export default {\n    project: {\n        name: \"auditpkg\",\n        version: \"0.1.0\"\n    },\n    license: \"MIT\"\n}\n",
    )
    .unwrap();
    let src = dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("main.rnx"), "fn Main(): Int {\n    return 40 + 2;\n}\n").unwrap();
    let lock = std::process::Command::new(rnx())
        .arg("lock")
        .current_dir(&dir)
        .output()
        .expect("run rnx lock");
    assert!(lock.status.success(), "rnx lock: {}", String::from_utf8_lossy(&lock.stderr));
    let mut s = Session::start_with_env(&[("RNX_REGISTRY", "http://127.0.0.1:9/")]);
    handshake(&mut s);
    let resp = s.call("inspect_package_capabilities", serde_json::json!({ "path": dir.to_str().unwrap() }));
    let out = Session::content_json(&resp);
    assert_eq!(out["status"], "ok", "{out:?}");
    assert!(out["capabilities"].is_object(), "{out:?}");
    assert!(out["drift"].as_array().expect("drift").is_empty(), "{out:?}");
    assert_eq!(out["advisories"]["status"], "skipped", "{out:?}");
    assert!(out["advisories"]["note"].as_str().unwrap_or("").contains("unavailable"), "{out:?}");
    let licensed = s.call(
        "inspect_package_capabilities",
        serde_json::json!({ "path": dir.to_str().unwrap(), "licenses": true, "deny_licenses": ["MIT"] }),
    );
    let licensed_json = Session::content_json(&licensed);
    assert_eq!(licensed_json["status"], "ok", "{licensed_json:?}");
    assert!(licensed_json["licenses"].as_str().unwrap_or("").contains("MIT"), "{licensed_json:?}");
    let denied = licensed_json["denied"].as_array().expect("denied");
    assert!(denied.iter().any(|d| d.as_str().unwrap_or("").contains("MIT")), "{licensed_json:?}");
    let manifest = s.call(
        "inspect_package_capabilities",
        serde_json::json!({ "path": dir.to_str().unwrap(), "export_manifest": true }),
    );
    let manifest_text = Session::content_text(&manifest);
    assert!(manifest_text.contains("auditpkg"), "{manifest_text:?}");
}

#[test]
fn mcp_build_snippet_and_project() {
    let dir = crate::scratch::dir("mcp-tools-build");
    write_project(&dir, "buildpkg", &[("main.rnx", MAIN_OK), ("math.rnx", MATH_LIB)]);
    let main = dir.join("src").join("main.rnx");
    let mut s = Session::start();
    handshake(&mut s);
    let snippet = s.call("build", serde_json::json!({ "source": "fn Main(): Int {\n    return 40 + 2;\n}\n" }));
    let snippet_json = Session::content_json(&snippet);
    assert_eq!(snippet_json["status"], "ok", "{snippet_json:?}");
    assert!(snippet_json["bytes"].as_u64().unwrap_or(0) > 0, "{snippet_json:?}");
    let broken = s.call("build", serde_json::json!({ "source": "fn Main(): Int {\n    return nope;\n}\n" }));
    let broken_json = Session::content_json(&broken);
    assert_eq!(broken_json["status"], "compile_error", "{broken_json:?}");
    assert!(!broken_json["errors"].as_array().expect("errors").is_empty(), "{broken_json:?}");
    let project = s.call("build", serde_json::json!({ "path": main.to_str().unwrap() }));
    let project_json = Session::content_json(&project);
    assert_eq!(project_json["status"], "ok", "{project_json:?}");
    assert!(project_json["bytes"].as_u64().unwrap_or(0) > 0, "{project_json:?}");
    let bad_opt = s.call("build", serde_json::json!({ "source": "fn Main(): Int {\n    return 1;\n}\n", "opt_level": 3 }));
    assert!(Session::is_error(&bad_opt), "{bad_opt:?}");
    let release = s.call("build", serde_json::json!({ "source": "fn Main(): Int {\n    return 40 + 2;\n}\n", "release": true }));
    let release_json = Session::content_json(&release);
    assert_eq!(release_json["status"], "ok", "{release_json:?}");
    assert_eq!(release_json["backend"], "llvm -O3 (release)", "{release_json:?}");
}

#[test]
fn mcp_test_pass_fail_and_filter() {
    let dir = crate::scratch::dir("mcp-tools-test");
    write_project(&dir, "testpkg", &[("main.rnx", "fn Main(): Int {\n    return 0;\n}\n")]);
    let tests = dir.join("tests");
    std::fs::create_dir_all(&tests).unwrap();
    std::fs::write(
        tests.join("basic.rnx"),
        "test fn adds_up() {\n    assert(1 + 1 == 2, \"math\");\n}\n\ntest fn falls_over() {\n    assert(1 == 2, \"broken\");\n}\n",
    )
    .unwrap();
    let mut s = Session::start();
    handshake(&mut s);
    let all = s.call("test", serde_json::json!({ "path": dir.to_str().unwrap() }));
    let all_json = Session::content_json(&all);
    assert_eq!(all_json["status"], "fail", "{all_json:?}");
    assert!(all_json["output"].as_str().unwrap_or("").contains("tests:"), "{all_json:?}");
    let filtered = s.call(
        "test",
        serde_json::json!({ "path": dir.to_str().unwrap(), "filter": "adds_up", "exact": true }),
    );
    let filtered_json = Session::content_json(&filtered);
    assert_eq!(filtered_json["status"], "pass", "{filtered_json:?}");
    let bad_backend = s.call("test", serde_json::json!({ "path": dir.to_str().unwrap(), "backend": "quantum" }));
    assert!(Session::is_error(&bad_backend), "{bad_backend:?}");
}

#[test]
fn mcp_eval_reset_type_and_size_cap() {
    let mut s = Session::start();
    handshake(&mut s);
    let def = s.call("eval_code", serde_json::json!({ "code": "let typed_var: Int = 6 * 7;" }));
    assert_eq!(def["result"]["isError"], false, "{def:?}");
    let val = s.call("eval_code", serde_json::json!({ "code": "typed_var" }));
    assert!(Session::content_text(&val).contains("42"), "{val:?}");
    let ty = s.call("eval_code", serde_json::json!({ "code": ":type typed_var" }));
    assert_eq!(ty["result"]["isError"], false, "{ty:?}");
    assert!(!Session::content_text(&ty).is_empty(), "{ty:?}");
    let reset = s.call("eval_code", serde_json::json!({ "code": "1 + 1", "reset": true }));
    assert_eq!(reset["result"]["isError"], false, "{reset:?}");
    let gone = s.call("eval_code", serde_json::json!({ "code": "typed_var" }));
    assert_eq!(gone["result"]["isError"], true, "{gone:?}");
    let big = "1".repeat(33 * 1024);
    let capped = s.call("eval_code", serde_json::json!({ "code": big }));
    assert!(Session::is_error(&capped), "{capped:?}");
}

#[test]
fn mcp_lookup_limit_and_resources_sharded() {
    let mut s = Session::start();
    handshake(&mut s);
    let limited = s.call("rasmalai_lookup_symbol", serde_json::json!({ "query": "a", "limit": 2 }));
    let limited_text = Session::content_text(&limited);
    assert!(limited_text.contains("match(es)"), "{limited_text:?}");
    let count: usize = limited_text.split_whitespace().next().and_then(|n| n.parse().ok()).unwrap_or(99);
    assert!(count <= 2, "{limited_text:?}");
    let missing = s.call("rasmalai_lookup_symbol", serde_json::json!({ "query": "NoSuchSymbolZZZ" }));
    assert!(Session::content_text(&missing).contains("no symbols matching"), "{missing:?}");
    let list = s.request("resources/list", serde_json::json!({}));
    let uris: Vec<String> = list["result"]["resources"]
        .as_array()
        .expect("resources array")
        .iter()
        .map(|r| r["uri"].as_str().expect("uri").to_string())
        .collect();
    assert!(uris.contains(&"rasmalai://spec/packages".to_string()), "{uris:?}");
    assert!(uris.contains(&"rasmalai://stdlib/bytes.json".to_string()), "{uris:?}");
    assert!(uris.contains(&"rasmalai://stdlib/api.json".to_string()), "{uris:?}");
    assert!(uris.len() >= 20, "{uris:?}");
    let shard = s.request("resources/read", serde_json::json!({ "uri": "rasmalai://stdlib/bytes.json" }));
    let shard_text = shard["result"]["contents"][0]["text"].as_str().expect("text");
    let full = s.request("resources/read", serde_json::json!({ "uri": "rasmalai://stdlib/api.json" }));
    let full_text = full["result"]["contents"][0]["text"].as_str().expect("text");
    assert!(shard_text.len() < full_text.len(), "shard must be smaller than the full catalog");
    let parsed: serde_json::Value = serde_json::from_str(shard_text).expect("shard parses");
    assert_eq!(parsed["modules"].as_array().expect("modules").len(), 1, "{parsed:?}");
    let unknown = s.request("resources/read", serde_json::json!({ "uri": "rasmalai://stdlib/nope.json" }));
    assert!(unknown.get("error").is_some(), "{unknown:?}");
}

#[test]
fn mcp_cold_start_walkthrough_uses_only_mcp_tools() {
    let dir = crate::scratch::dir("mcp-tools-coldstart");
    write_project(
        &dir,
        "coldstart",
        &[(
            "main.rnx",
            "import { double } from \"./math\";\nimport { Vec2 } from \"@std/math\";\n\nfn Main(): Int {\n    let v = new Vec2(3.0, 4.0);\n    print(v.length());\n    return double(21);\n}\n",
        ), ("math.rnx", MATH_LIB)],
    );
    let tests = dir.join("tests");
    std::fs::create_dir_all(&tests).unwrap();
    std::fs::write(
        tests.join("basic.rnx"),
        "test fn doubling() {\n    assert(2 * 21 == 42, \"math\");\n}\n",
    )
    .unwrap();
    let main = dir.join("src").join("main.rnx");
    let main_str = main.to_str().unwrap();
    let dir_str = dir.to_str().unwrap();
    let mut s = Session::start();
    handshake(&mut s);
    let lookup = s.call("rasmalai_lookup_symbol", serde_json::json!({ "query": "Vec2" }));
    assert!(Session::content_text(&lookup).contains("Vec2"), "{lookup:?}");
    let check = s.call("check", serde_json::json!({ "path": main_str }));
    assert_eq!(Session::content_json(&check)["status"], "ok", "{check:?}");
    let diag = s.call("get_diagnostics", serde_json::json!({ "path": main_str }));
    assert_eq!(Session::content_json(&diag)["status"], "ok", "{diag:?}");
    let run = s.call("run", serde_json::json!({ "path": main_str }));
    let run_json = Session::content_json(&run);
    assert_eq!(run_json["status"], "value", "{run_json:?}");
    assert_eq!(run_json["value"], "42", "{run_json:?}");
    let fmt = s.call("fmt", serde_json::json!({ "path": main_str, "check": true }));
    assert!(Session::content_text(&fmt).contains("already formatted"), "{fmt:?}");
    let lint = s.call("lint", serde_json::json!({ "path": dir_str }));
    assert_eq!(Session::content_json(&lint)["status"], "ok", "{lint:?}");
    let doc = s.call("doc", serde_json::json!({ "path": dir_str }));
    assert_eq!(Session::content_json(&doc)["status"], "ok", "{doc:?}");
    let test = s.call("test", serde_json::json!({ "path": dir_str }));
    assert_eq!(Session::content_json(&test)["status"], "pass", "{test:?}");
    let build = s.call("build", serde_json::json!({ "path": main_str }));
    assert_eq!(Session::content_json(&build)["status"], "ok", "{build:?}");
    let version = s.call("version", serde_json::json!({}));
    assert!(Session::content_text(&version).contains("rnx "), "{version:?}");
    let explain = s.call("explain", serde_json::json!({ "code": "e108" }));
    assert!(Session::content_text(&explain).contains("E108"), "{explain:?}");
}

#[test]
fn mcp_dead_registry_check_stays_bounded_and_offline_tolerant() {
    let cache = crate::scratch::dir("mcp-tools-deadcache");
    let mut s = Session::start_with_env(&[
        ("RNX_REGISTRY", "http://127.0.0.1:9/"),
        ("RNX_CACHE_HOME", cache.to_str().unwrap()),
    ]);
    handshake(&mut s);
    let resp = s.call(
        "check",
        serde_json::json!({ "source": "import { thing } from \"no-such-pkg-zzz\";\n\nfn Main(): Int {\n    return 0;\n}\n" }),
    );
    let text = Session::content_text(&resp);
    let out: serde_json::Value = serde_json::from_str(&text).expect("check returns JSON");
    assert_eq!(out["status"], "error", "{out:?}");
    let errors = out["errors"].as_array().expect("errors");
    assert!(!errors.is_empty(), "{out:?}");
    let codes: Vec<String> = errors
        .iter()
        .filter_map(|e| e["code"].as_str().map(|c| c.to_string()))
        .collect();
    assert!(
        codes.iter().any(|c| c == "E108" || c == "E113"),
        "offline failures must be E108/E113 with recovery hints: {out:?}"
    );
    for e in errors {
        if e["code"] == "E113" {
            assert!(
                e["hint"].as_str().unwrap_or("").contains("rnx download"),
                "E113 must name the recovery: {e:?}"
            );
        }
    }
}
