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
        let mut buf = vec![0u8; content_length.unwrap()];
        self.stdout.read_exact(&mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("invalid frame {e}: {text}"))
    }

    fn initialize(&mut self) {
        self.send("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}");
        let response = self.recv();
        assert!(response["result"]["capabilities"]["documentSymbolProvider"].as_bool().unwrap_or(false), "{response}");
        assert!(response["result"]["capabilities"]["completionProvider"]["triggerCharacters"][0] == serde_json::Value::String(".".to_string()), "{response}");
        self.send("{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}");
    }

    fn did_open(&mut self, uri: &str, text: &str) {
        let body = format!(
            "{{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/didOpen\",\"params\":{{\"textDocument\":{{\"uri\":\"{uri}\",\"text\":{}}}}}}}",
            escape_for_test(text)
        );
        self.send(&body);
        let _ = self.recv();
    }

    fn shutdown(&mut self) {
        self.send("{\"jsonrpc\":\"2.0\",\"id\":99,\"method\":\"shutdown\",\"params\":null}");
        let _ = self.recv();
        self.send("{\"jsonrpc\":\"2.0\",\"method\":\"exit\"}");
    }

    fn stop(mut self, expect_code: i32) {
        drop(self.stdin);
        let status = self.child.wait().unwrap();
        assert_eq!(status.code(), Some(expect_code), "lsp exit code");
    }
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

fn labels(items: &serde_json::Value) -> Vec<String> {
    items
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(|i| i["label"].as_str().map(|s| s.to_string()))
        .collect()
}

fn kind_of(items: &serde_json::Value, label: &str) -> Option<u64> {
    items
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .find(|i| i["label"] == serde_json::Value::String(label.to_string()))
        .and_then(|i| i["kind"].as_u64())
}

const SYMBOL_SRC: &str = "class Scene {\n    let name: String;\n    init(name: String) { this.name = name; }\n    spawn(): Int {\n        return 1;\n    }\n}\n\nfn helper(): Int {\n    return 1;\n}\n\ntest fn check_math() {\n    assert(1 + 1 == 2, \"ok\");\n}\n";

#[test]
fn test_lsp_document_symbols_hierarchy() {
    let mut session = Session::start();
    session.initialize();
    session.did_open("file:///symbols.rnx", SYMBOL_SRC);
    session.send("{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"textDocument/documentSymbol\",\"params\":{\"textDocument\":{\"uri\":\"file:///symbols.rnx\"}}}");
    let response = session.recv();
    let symbols = response["result"].as_array().unwrap_or_else(|| panic!("no symbols: {response}"));
    let class = symbols.iter().find(|s| s["name"] == serde_json::Value::String("Scene".to_string())).unwrap_or_else(|| panic!("no Scene: {response}"));
    assert_eq!(class["kind"], serde_json::Value::from(5), "{response}");
    let children = class["children"].as_array().unwrap_or_else(|| panic!("no children: {response}"));
    let field = children.iter().find(|c| c["name"] == serde_json::Value::String("name".to_string())).unwrap_or_else(|| panic!("no field: {response}"));
    assert_eq!(field["kind"], serde_json::Value::from(8), "{response}");
    let method = children.iter().find(|c| c["name"] == serde_json::Value::String("spawn".to_string())).unwrap_or_else(|| panic!("no method: {response}"));
    assert_eq!(method["kind"], serde_json::Value::from(6), "{response}");
    let func = symbols.iter().find(|s| s["name"] == serde_json::Value::String("helper".to_string())).unwrap_or_else(|| panic!("no helper: {response}"));
    assert_eq!(func["kind"], serde_json::Value::from(12), "{response}");
    let test = symbols.iter().find(|s| s["name"] == serde_json::Value::String("check_math".to_string())).unwrap_or_else(|| panic!("no test: {response}"));
    assert_eq!(test["kind"], serde_json::Value::from(12), "{response}");
    assert_eq!(test["detail"], serde_json::Value::String("test".to_string()), "{response}");
    session.shutdown();
    session.stop(0);
}

const BARE_SRC: &str = "fn demo(paramVal: Int): Void {\n    let localVar = 10;\n}\n";

#[test]
fn test_lsp_completion_bare_identifier() {
    let mut session = Session::start();
    session.initialize();
    session.did_open("file:///bare.rnx", BARE_SRC);
    session.send("{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"textDocument/completion\",\"params\":{\"textDocument\":{\"uri\":\"file:///bare.rnx\"},\"position\":{\"line\":1,\"character\":20}}}");
    let response = session.recv();
    let items = &response["result"]["items"];
    assert_eq!(kind_of(items, "localVar"), Some(6), "{response}");
    assert_eq!(kind_of(items, "paramVal"), Some(6), "{response}");
    assert!(labels(items).contains(&"return".to_string()), "{response}");
    session.shutdown();
    session.stop(0);
}

const MEMBER_SRC: &str = "import { Vec4f } from \"@std/simd\";\nfn demo(v: Vec4f): Void {\n    v.\n}\n";

#[test]
fn test_lsp_completion_member_dot_access() {
    let mut session = Session::start();
    session.initialize();
    session.did_open("file:///member.rnx", MEMBER_SRC);
    session.send("{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"textDocument/completion\",\"params\":{\"textDocument\":{\"uri\":\"file:///member.rnx\"},\"position\":{\"line\":2,\"character\":6}}}");
    let response = session.recv();
    let items = &response["result"]["items"];
    let names = labels(items);
    for lane in ["x", "y", "z", "w"] {
        assert!(names.contains(&lane.to_string()), "{response}");
    }
    session.shutdown();
    session.stop(0);
}

#[test]
fn test_lsp_completion_keywords_and_types() {
    let mut session = Session::start();
    session.initialize();
    session.did_open("file:///empty.rnx", "");
    session.send("{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"textDocument/completion\",\"params\":{\"textDocument\":{\"uri\":\"file:///empty.rnx\"},\"position\":{\"line\":0,\"character\":0}}}");
    let response = session.recv();
    let items = &response["result"]["items"];
    let names = labels(items);
    for expected in ["pub", "fn", "class", "Vec4f", "Int"] {
        assert!(names.contains(&expected.to_string()), "{response}");
    }
    session.shutdown();
    session.stop(0);
}
