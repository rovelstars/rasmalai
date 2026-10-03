use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-wave3-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn e303_caret_points_at_offending_token() {
    let dir = fresh_dir("e303");
    let file = dir.join("main.rnx");
    let src = "fn main(): Int { let x = nosuchvar + 1; return x; }\n";
    std::fs::write(&file, src).unwrap();
    let out = Command::new(rnx()).arg("check").arg(&file).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8(out.stderr).unwrap();
    let line = src.lines().next().unwrap();
    let col = line.find("nosuchvar").unwrap() + 1;
    assert_eq!(col, 26);
    assert!(
        stderr.contains(&format!("main.rnx:1:{col}")),
        "caret must report the token column, got:\n{stderr}"
    );
    assert!(!stderr.contains("main.rnx:1:18"), "stale declaration-anchored span:\n{stderr}");
    let excerpt = stderr.lines().find(|l| l.contains("nosuchvar + 1")).unwrap().to_string();
    let caret = stderr.lines().find(|l| l.contains('▲')).unwrap().to_string();
    let token_off = excerpt.match_indices("nosuchvar").next().map(|(b, _)| {
        excerpt[..b].chars().count()
    }).unwrap();
    let caret_off = caret.match_indices('▲').next().map(|(b, _)| {
        caret[..b].chars().count()
    }).unwrap();
    assert_eq!(
        token_off, caret_off,
        "caret must sit under the token:\n{excerpt}\n{caret}"
    );
    let _ = std::fs::remove_dir_all(&dir);
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
        let mut s = Session { child, next_id: 1, reader: BufReader::new(stdout) };
        let init = s.request(
            "initialize",
            serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "wave3", "version": "0.0.0" },
            }),
        );
        assert_eq!(
            init["result"]["serverInfo"]["name"], "rnx-mcp",
            "guide promises serverInfo.name rnx-mcp: {init:?}"
        );
        s.notify("notifications/initialized");
        s
    }

    fn request(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        let id = self.next_id;
        self.next_id += 1;
        let msg = serde_json::json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
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
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

const EXPECTED_TOOLS: &[&str] = &[
    "check",
    "run",
    "fmt",
    "explain",
    "rasmalai_lookup_symbol",
    "eval_code",
    "hot_reload",
    "get_diagnostics",
    "version",
    "inspect_package_capabilities",
];

#[test]
fn mcp_tools_list_matches_documentation() {
    let mut s = Session::start();
    let resp = s.request("tools/list", serde_json::json!({}));
    let mut names: Vec<String> = resp["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|t| t["name"].as_str().expect("name").to_string())
        .collect();
    names.sort();
    let mut expected: Vec<String> = EXPECTED_TOOLS.iter().map(|n| n.to_string()).collect();
    expected.sort();
    assert_eq!(names, expected, "tools/list must match the documented set");
}

#[test]
fn mcp_prompts_teach_new_for_classes() {
    let mut s = Session::start();
    let expert = s.request("prompts/get", serde_json::json!({ "name": "rasmalai-expert" }));
    let text = serde_json::to_string(&expert).unwrap();
    assert!(text.contains("new Meter"), "expert prompt must teach `new`: {text}");
    assert!(text.contains("E204"), "expert prompt must name the direct-call error: {text}");
    assert!(!text.contains("NO `new` keyword"), "stale rule inversion: {text}");
    assert!(!text.contains("(no `new`)"), "stale rule inversion: {text}");
    let convert = s.request(
        "prompts/get",
        serde_json::json!({
            "name": "convert-to-rasmalai",
            "arguments": { "source_language": "rust", "code": "struct S;" },
        }),
    );
    let text = serde_json::to_string(&convert).unwrap();
    assert!(text.contains("new Class"), "convert prompt must teach `new`: {text}");
    assert!(
        !text.contains("replace `new` construction with direct calls"),
        "stale rule inversion: {text}"
    );
}

const EXPECTED_MODULES: &[&str] = &[
    "prelude", "simd", "math", "collections", "fs", "bytes", "time", "random", "sync", "env",
    "process", "os", "testing", "web", "json", "net", "io",
];

#[test]
fn stdlib_module_list_matches_docs() {
    let mut shipped: Vec<String> = std::fs::read_dir(crate_dir().join("../stdlib/src"))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".rnx"))
        .map(|n| n.trim_end_matches(".rnx").to_string())
        .collect();
    shipped.sort();
    let mut expected: Vec<String> = EXPECTED_MODULES.iter().map(|n| n.to_string()).collect();
    expected.sort();
    assert_eq!(shipped, expected, "stdlib modules on disk");
    let manual = std::fs::read_to_string(
        crate_dir().join("../../website/src/content/manual/16-project-and-toolchain.md"),
    )
    .unwrap();
    for m in EXPECTED_MODULES {
        assert!(manual.contains(m), "manual/16 must list @std/{m}");
    }
    assert!(manual.contains("Seventeen modules"), "manual/16 must count seventeen");
    let stdlib_ts =
        std::fs::read_to_string(crate_dir().join("../../website/src/lib/docs/stdlib.ts")).unwrap();
    for m in EXPECTED_MODULES {
        assert!(stdlib_ts.contains(&format!("name: '{m}'")), "stdlib.ts must list {m}");
    }
}

#[test]
fn init_scaffold_check_and_run_clean() {
    let dir = fresh_dir("init");
    let name = "hello";
    let init = Command::new(rnx()).arg("init").arg(name).current_dir(&dir).output().unwrap();
    assert!(init.status.success(), "{}", String::from_utf8_lossy(&init.stderr));
    let root = dir.join(name);
    let main = std::fs::read_to_string(root.join("src/main.rnx")).unwrap();
    assert!(main.contains("fn Main(): Int"), "guide shows the named entry point:\n{main}");
    assert!(main.contains(&format!("Hello from {name}!")), "guide promises the greeting:\n{main}");
    let check = Command::new(rnx()).arg("check").current_dir(&root).output().unwrap();
    assert!(check.status.success(), "{}", String::from_utf8_lossy(&check.stderr));
    let run = Command::new(rnx()).arg("run").current_dir(&root).output().unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert_eq!(
        String::from_utf8(run.stdout).unwrap(),
        format!("Hello from {name}!\n"),
        "first run prints the documented line"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
