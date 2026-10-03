use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

struct Session {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Session {
    fn start() -> Session {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rnx"))
            .arg("lsp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Session { child, stdin, stdout }
    }

    fn send(&mut self, body: &str) {
        write!(self.stdin, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        self.stdin.flush().unwrap();
    }

    fn recv(&mut self) -> serde_json::Value {
        let mut content_length: Option<usize> = None;
        loop {
            let mut line = String::new();
            self.stdout.read_line(&mut line).unwrap();
            let trimmed = line.trim();
            if trimmed.is_empty() {
                break;
            }
            if let Some(value) = trimmed.strip_prefix("Content-Length:") {
                content_length = value.trim().parse::<usize>().ok();
            }
        }
        let length = content_length.unwrap();
        let mut buf = vec![0u8; length];
        self.stdout.read_exact(&mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("invalid frame {e}: {text}"))
    }

    fn stop(mut self, expect_code: i32) {
        self.stdin.flush().unwrap();
        drop(self.stdin);
        let status = self.child.wait().unwrap();
        assert_eq!(status.code(), Some(expect_code), "lsp exit code");
    }
}

fn initialize(session: &mut Session) {
    session.send("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}");
    let response = session.recv();
    assert_eq!(response["result"]["capabilities"]["textDocumentSync"], serde_json::Value::from(1), "{response}");
    session.send("{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}");
}

fn shutdown(session: &mut Session) {
    session.send("{\"jsonrpc\":\"2.0\",\"id\":99,\"method\":\"shutdown\",\"params\":null}");
    let response = session.recv();
    assert!(response["result"].is_null(), "{response}");
    session.send("{\"jsonrpc\":\"2.0\",\"method\":\"exit\"}");
}

fn did_open(session: &mut Session, uri: &str, text: &str) -> serde_json::Value {
    let body = format!(
        "{{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/didOpen\",\"params\":{{\"textDocument\":{{\"uri\":\"{uri}\",\"text\":{}}}}}}}",
        escape_for_test(text)
    );
    session.send(&body);
    session.recv()
}

fn escape_for_test(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

#[test]
fn test_lsp_initialize_and_shutdown_lifecycle() {
    let mut session = Session::start();
    initialize(&mut session);
    shutdown(&mut session);
    session.stop(0);
}

const AWAIT_SRC: &str = "fn Main(): Int {\n    await foo();\n    return 0;\n}\n";

#[test]
fn test_lsp_did_open_publishes_diagnostics() {
    let mut session = Session::start();
    initialize(&mut session);
    let message = did_open(&mut session, "file:///await.rnx", AWAIT_SRC);
    assert_eq!(message["method"], serde_json::Value::String("textDocument/publishDiagnostics".to_string()), "{message}");
    let diags = message["params"]["diagnostics"].as_array().unwrap_or_else(|| panic!("no diagnostics: {message}"));
    assert!(!diags.is_empty(), "{message}");
    let diag = diags.iter().find(|d| d["code"] == serde_json::Value::String("E109".to_string())).unwrap_or_else(|| panic!("no E109: {message}"));
    assert_eq!(diag["severity"], serde_json::Value::from(1), "{message}");
    assert_eq!(diag["source"], serde_json::Value::String("rasmalai".to_string()), "{message}");
    let start = &diag["range"]["start"];
    assert!(start["line"].as_u64().is_some(), "{message}");
    assert!(start["character"].as_u64().is_some(), "{message}");
    shutdown(&mut session);
    session.stop(0);
}

const VALID_SRC: &str = "fn Main(): Int {\n    return 0;\n}\n";

#[test]
fn test_lsp_did_change_clears_diagnostics() {
    let mut session = Session::start();
    initialize(&mut session);
    let opened = did_open(&mut session, "file:///change.rnx", AWAIT_SRC);
    assert!(!opened["params"]["diagnostics"].as_array().unwrap().is_empty());
    let body = format!(
        "{{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/didChange\",\"params\":{{\"textDocument\":{{\"uri\":\"file:///change.rnx\"}},\"contentChanges\":[{{\"text\":{}}}]}}}}",
        escape_for_test(VALID_SRC)
    );
    session.send(&body);
    let message = session.recv();
    assert_eq!(message["method"], serde_json::Value::String("textDocument/publishDiagnostics".to_string()), "{message}");
    let diags = message["params"]["diagnostics"].as_array().unwrap_or_else(|| panic!("no array: {message}"));
    assert!(diags.is_empty(), "{message}");
    shutdown(&mut session);
    session.stop(0);
}

const UNUSED_SRC: &str = "fn Main(): Int {\n    let unused = 42;\n    return 0;\n}\n";

#[test]
fn test_lsp_did_open_reports_lint_warnings() {
    let mut session = Session::start();
    initialize(&mut session);
    let message = did_open(&mut session, "file:///unused.rnx", UNUSED_SRC);
    let diags = message["params"]["diagnostics"].as_array().unwrap_or_else(|| panic!("no diagnostics: {message}"));
    let diag = diags.iter().find(|d| d["code"] == serde_json::Value::String("L001".to_string())).unwrap_or_else(|| panic!("no L001: {message}"));
    assert_eq!(diag["severity"], serde_json::Value::from(2), "{message}");
    shutdown(&mut session);
    session.stop(0);
}

fn e105_diag(message: &serde_json::Value) -> serde_json::Value {
    let diags = message["params"]["diagnostics"]
        .as_array()
        .unwrap_or_else(|| panic!("no diagnostics: {message}"));
    diags
        .iter()
        .find(|d| d["code"] == serde_json::Value::String("E105".to_string()))
        .unwrap_or_else(|| panic!("no E105: {message}"))
        .clone()
}

fn code_actions(
    session: &mut Session,
    id: u64,
    uri: &str,
    diag: &serde_json::Value,
) -> serde_json::Value {
    let range = diag["range"].to_string();
    let body = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"textDocument/codeAction\",\"params\":{{\"textDocument\":{{\"uri\":\"{uri}\"}},\"range\":{range},\"context\":{{\"diagnostics\":[{diag}]}}}}}}"
    );
    session.send(&body);
    session.recv()
}

const ANON_ARROW_SRC: &str = "fn Main(): Int {\n  let mul = fn(x) => x * 2;\n  return mul(21);\n}\n";

#[test]
fn test_lsp_code_action_converts_fn_lambda_to_arrow() {
    let mut session = Session::start();
    initialize(&mut session);
    let uri = "file:///fix_arrow.rnx";
    let opened = did_open(&mut session, uri, ANON_ARROW_SRC);
    let diag = e105_diag(&opened);
    let response = code_actions(&mut session, 10, uri, &diag);
    let actions = response["result"].as_array().unwrap_or_else(|| panic!("no actions: {response}"));
    assert_eq!(actions.len(), 1, "{response}");
    assert_eq!(actions[0]["title"], serde_json::Value::String("Convert to arrow lambda".to_string()), "{response}");
    let edits = actions[0]["edit"]["changes"][uri].as_array().unwrap_or_else(|| panic!("no edits: {response}"));
    assert_eq!(edits.len(), 1, "{response}");
    assert_eq!(edits[0]["newText"], serde_json::Value::String(String::new()), "{response}");
    assert_eq!(edits[0]["range"]["start"]["character"], serde_json::Value::from(12), "{response}");
    assert_eq!(edits[0]["range"]["end"]["character"], serde_json::Value::from(14), "{response}");
    shutdown(&mut session);
    session.stop(0);
}

const NAMED_ARROW_SRC: &str = "fn foo(x: Int): Int => x * 2;\n";

#[test]
fn test_lsp_code_action_converts_named_arrow_to_block() {
    let mut session = Session::start();
    initialize(&mut session);
    let uri = "file:///fix_block.rnx";
    let opened = did_open(&mut session, uri, NAMED_ARROW_SRC);
    let diag = e105_diag(&opened);
    let response = code_actions(&mut session, 11, uri, &diag);
    let actions = response["result"].as_array().unwrap_or_else(|| panic!("no actions: {response}"));
    assert_eq!(actions.len(), 1, "{response}");
    assert_eq!(actions[0]["title"], serde_json::Value::String("Convert to block body".to_string()), "{response}");
    let edits = actions[0]["edit"]["changes"][uri].as_array().unwrap_or_else(|| panic!("no edits: {response}"));
    let new_text = edits[0]["newText"].as_str().unwrap_or_else(|| panic!("no newText: {response}"));
    assert!(new_text.contains("return x * 2;"), "{response}");
    let start = edits[0]["range"]["start"]["character"].as_u64().unwrap() as usize;
    let end = edits[0]["range"]["end"]["character"].as_u64().unwrap() as usize;
    let fixed = format!("{}{}{}", &NAMED_ARROW_SRC[..start], new_text, &NAMED_ARROW_SRC[end..]);
    frontend::parser::Parser::parse_module(&fixed)
        .unwrap_or_else(|e| panic!("quick fix result fails to parse: {e}\n{fixed}"));
    shutdown(&mut session);
    session.stop(0);
}

const PARAM_SRC: &str = "fn F(a: Int): Int {\n  let g = (x) => x + a;\n  return g(1);\n}\n";

#[test]
fn test_lsp_semantic_tokens_mark_parameters() {
    let mut session = Session::start();
    initialize(&mut session);
    did_open(&mut session, "file:///params.rnx", PARAM_SRC);
    session.send("{\"jsonrpc\":\"2.0\",\"id\":12,\"method\":\"textDocument/semanticTokens/full\",\"params\":{\"textDocument\":{\"uri\":\"file:///params.rnx\"}}}");
    let response = session.recv();
    let data = response["result"]["data"].as_array().unwrap_or_else(|| panic!("no data: {response}"));
    let nums: Vec<u64> = data.iter().map(|v| v.as_u64().unwrap()).collect();
    assert_eq!(nums, vec![0, 5, 1, 0, 0, 1, 11, 1, 0, 0], "{response}");
    shutdown(&mut session);
    session.stop(0);
}

#[test]
fn test_lsp_advertises_action_and_token_capabilities() {
    let mut session = Session::start();
    session.send("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}");
    let response = session.recv();
    let caps = &response["result"]["capabilities"];
    assert_eq!(caps["codeActionProvider"], serde_json::Value::Bool(true), "{response}");
    assert_eq!(
        caps["semanticTokensProvider"]["legend"]["tokenTypes"],
        serde_json::json!(["parameter"]),
        "{response}"
    );
    assert_eq!(caps["semanticTokensProvider"]["full"], serde_json::Value::Bool(true), "{response}");
    session.send("{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}");
    shutdown(&mut session);
    session.stop(0);
}

const TWO_BAD_SRC: &str = "fn a(): Int => 1;\nfn b(): Int => 2;\n";

#[test]
fn test_lsp_publishes_all_errors_in_file() {
    let mut session = Session::start();
    initialize(&mut session);
    let message = did_open(&mut session, "file:///two.rnx", TWO_BAD_SRC);
    let diags = message["params"]["diagnostics"].as_array().unwrap_or_else(|| panic!("no diagnostics: {message}"));
    let e105: Vec<_> = diags
        .iter()
        .filter(|d| {
            d["code"] == serde_json::Value::String("E105".to_string())
                && d["severity"] == serde_json::Value::from(1)
        })
        .collect();
    assert_eq!(e105.len(), 2, "{message}");
    shutdown(&mut session);
    session.stop(0);
}
