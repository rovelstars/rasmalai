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
        assert!(response["result"]["capabilities"]["hoverProvider"].as_bool().unwrap_or(false), "{response}");
        assert!(response["result"]["capabilities"]["definitionProvider"].as_bool().unwrap_or(false), "{response}");
        assert!(response["result"]["capabilities"]["documentFormattingProvider"].as_bool().unwrap_or(false), "{response}");
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

const UNFORMATTED: &str = "fn add(a:Int,b:Int):Int{return a+b;}\n";
const DOC_SRC: &str = "/** Adds two integers. */\nfn add(a: Int, b: Int): Int { return a + b; }\nfn Main(): Int { return add(1, 2); }\n";
const VAR_SRC: &str = "fn Main(): Int {\n    let total = 42;\n    return total;\n}\n";
const TRAILING_SPACED: &str = "fn Main(): Int {\n    let x = 1    + 2;\n    return x;\n}\n";

fn offset_of(src: &str, line: usize, character: usize) -> usize {
    let mut offset = 0usize;
    for (current, text) in src.split_inclusive('\n').enumerate() {
        if current == line {
            for (col, ch) in text.chars().enumerate() {
                if col >= character || ch == '\n' {
                    break;
                }
                offset += ch.len_utf8();
            }
            return offset;
        }
        offset += text.len();
    }
    src.len()
}

fn apply_edit(src: &str, edit: &serde_json::Value) -> String {
    let start = offset_of(
        src,
        edit["range"]["start"]["line"].as_u64().unwrap() as usize,
        edit["range"]["start"]["character"].as_u64().unwrap() as usize,
    );
    let end = offset_of(
        src,
        edit["range"]["end"]["line"].as_u64().unwrap() as usize,
        edit["range"]["end"]["character"].as_u64().unwrap() as usize,
    );
    let mut out = src[..start].to_string();
    out.push_str(edit["newText"].as_str().unwrap_or(""));
    out.push_str(&src[end..]);
    out
}

#[test]
fn test_lsp_formatting_request() {
    let mut session = Session::start();
    session.initialize();
    session.did_open("file:///fmt.rnx", UNFORMATTED);
    session.send("{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"textDocument/formatting\",\"params\":{\"textDocument\":{\"uri\":\"file:///fmt.rnx\"},\"options\":{}}}");
    let response = session.recv();
    let edits = response["result"].as_array().unwrap_or_else(|| panic!("no edits: {response}"));
    assert_eq!(edits.len(), 1, "{response}");
    let new_text = edits[0]["newText"].as_str().unwrap_or("");
    assert!(new_text.contains("fn add(a: Int, b: Int): Int {"), "{response}");
    assert!(new_text.contains("    return a + b;"), "{response}");
    assert_eq!(edits[0]["range"]["start"]["line"], serde_json::Value::from(0), "{response}");
    session.shutdown();
    session.stop(0);
}

#[test]
fn test_lsp_formatting_no_extra_trailing_newline() {
    let mut session = Session::start();
    session.initialize();
    session.did_open("file:///trail.rnx", TRAILING_SPACED);
    session.send("{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"textDocument/formatting\",\"params\":{\"textDocument\":{\"uri\":\"file:///trail.rnx\"},\"options\":{}}}");
    let response = session.recv();
    let edits = response["result"].as_array().unwrap_or_else(|| panic!("no edits: {response}"));
    assert_eq!(edits.len(), 1, "{response}");
    let applied = apply_edit(TRAILING_SPACED, &edits[0]);
    let expected = "fn Main(): Int {\n    let x = 1 + 2;\n    return x;\n}\n";
    assert_eq!(applied, expected, "{response}");
    session.shutdown();
    session.stop(0);
}

#[test]
fn test_lsp_hover_function_with_docstring() {
    let mut session = Session::start();
    session.initialize();
    session.did_open("file:///hover.rnx", DOC_SRC);
    session.send("{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"textDocument/hover\",\"params\":{\"textDocument\":{\"uri\":\"file:///hover.rnx\"},\"position\":{\"line\":2,\"character\":25}}}");
    let response = session.recv();
    let value = response["result"]["contents"]["value"].as_str().unwrap_or("");
    assert!(value.contains("fn add(a: Int, b: Int): Int"), "{response}");
    assert!(value.contains("Adds two integers."), "{response}");
    session.shutdown();
    session.stop(0);
}

#[test]
fn test_lsp_hover_local_variable() {
    let mut session = Session::start();
    session.initialize();
    session.did_open("file:///var.rnx", VAR_SRC);
    session.send("{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"textDocument/hover\",\"params\":{\"textDocument\":{\"uri\":\"file:///var.rnx\"},\"position\":{\"line\":2,\"character\":12}}}");
    let response = session.recv();
    let value = response["result"]["contents"]["value"].as_str().unwrap_or("");
    assert!(value.contains("let total: Int"), "{response}");
    session.shutdown();
    session.stop(0);
}

#[test]
fn test_lsp_definition_jump_to_function() {
    let mut session = Session::start();
    session.initialize();
    session.did_open("file:///def.rnx", DOC_SRC);
    session.send("{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"textDocument/definition\",\"params\":{\"textDocument\":{\"uri\":\"file:///def.rnx\"},\"position\":{\"line\":2,\"character\":25}}}");
    let response = session.recv();
    let range = &response["result"]["range"];
    assert_eq!(range["start"]["line"], serde_json::Value::from(1), "{response}");
    assert_eq!(range["start"]["character"], serde_json::Value::from(3), "{response}");
    assert_eq!(range["end"]["character"], serde_json::Value::from(6), "{response}");
    assert!(response["result"]["uri"].as_str().unwrap_or("").ends_with("def.rnx"), "{response}");
    session.shutdown();
    session.stop(0);
}

#[test]
fn test_lsp_definition_jump_to_parameter() {
    let mut session = Session::start();
    session.initialize();
    session.did_open("file:///param.rnx", DOC_SRC);
    session.send("{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"textDocument/definition\",\"params\":{\"textDocument\":{\"uri\":\"file:///param.rnx\"},\"position\":{\"line\":1,\"character\":37}}}");
    let response = session.recv();
    let range = &response["result"]["range"];
    assert_eq!(range["start"]["line"], serde_json::Value::from(1), "{response}");
    assert_eq!(range["start"]["character"], serde_json::Value::from(7), "{response}");
    assert_eq!(range["end"]["character"], serde_json::Value::from(8), "{response}");
    session.shutdown();
    session.stop(0);
}

const CLASS_SRC: &str = "/** Point in space. */\nclass Vec {\n    let x: Float;\n    init(x: Float) {\n        this.x = x;\n    }\n}\n";

fn hover_value(session: &mut Session, uri: &str, line: u64, character: u64) -> Option<String> {
    session.send(&format!("{{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"textDocument/hover\",\"params\":{{\"textDocument\":{{\"uri\":\"{uri}\"}},\"position\":{{\"line\":{line},\"character\":{character}}}}}}}"));
    let response = session.recv();
    response["result"]["contents"]["value"].as_str().map(|s| s.to_string())
}

#[test]
fn test_lsp_hover_member_field_and_init() {
    let mut session = Session::start();
    session.initialize();
    session.did_open("file:///class.rnx", CLASS_SRC);
    let field = hover_value(&mut session, "file:///class.rnx", 4, 13).unwrap_or_else(|| panic!("no member hover"));
    assert!(field.contains("let x: Float"), "{field}");
    let use_site = hover_value(&mut session, "file:///class.rnx", 4, 17).unwrap_or_else(|| panic!("no use hover"));
    assert!(use_site.contains("x: Float"), "{use_site}");
    let init = hover_value(&mut session, "file:///class.rnx", 3, 4).unwrap_or_else(|| panic!("no init hover"));
    assert!(init.contains("init(x: Float)"), "{init}");
    let this = hover_value(&mut session, "file:///class.rnx", 4, 9).unwrap_or_else(|| panic!("no this hover"));
    assert!(this.contains("class Vec"), "{this}");
    session.shutdown();
    session.stop(0);
}

#[test]
fn test_lsp_hover_jsdoc_comment() {
    let mut session = Session::start();
    session.initialize();
    session.did_open("file:///jsdoc.rnx", "/** Adds one to x. */\nfn inc(x: Int): Int {\n    return x;\n}\nfn Main(): Int {\n    return inc(41);\n}\n");
    let value = hover_value(&mut session, "file:///jsdoc.rnx", 5, 12).unwrap_or_else(|| panic!("no hover"));
    assert!(value.contains("fn inc(x: Int): Int"), "{value}");
    assert!(value.contains("Adds one to x."), "{value}");
    session.shutdown();
    session.stop(0);
}

#[test]
fn test_lsp_hover_interface_docs() {
    let mut session = Session::start();
    session.initialize();
    session.did_open("file:///iface.rnx", "/** Drawable shape. */\ninterface Shape {\n    /** Draw it. */\n    fn draw(): Int;\n}\nfn Main(): Int {\n    return 0;\n}\n");
    let decl = hover_value(&mut session, "file:///iface.rnx", 1, 11).unwrap_or_else(|| panic!("no iface hover"));
    assert!(decl.contains("interface Shape"), "{decl}");
    assert!(decl.contains("Drawable shape."), "{decl}");
    let method = hover_value(&mut session, "file:///iface.rnx", 3, 7).unwrap_or_else(|| panic!("no method hover"));
    assert!(method.contains("fn draw(): Int"), "{method}");
    assert!(method.contains("Draw it."), "{method}");
    session.shutdown();
    session.stop(0);
}

const CONFIG_SRC: &str = "export default {\n    project: {\n        name:  \"demo\",\n        version: \"1.0\",\n        bogus: 1\n    }\n}\n";

#[test]
fn test_lsp_config_diagnostics_hover_completion_formatting() {
    let mut session = Session::start();
    session.initialize();
    session.did_open("file:///Project.config", CONFIG_SRC);
    session.send("{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/didChange\",\"params\":{\"textDocument\":{\"uri\":\"file:///Project.config\"},\"contentChanges\":[{\"text\":\"export default {\\n    project: {\\n        name:  \\\"demo\\\",\\n        version: \\\"1.0\\\",\\n        bogus: 1\\n    }\\n}\\n\"}]}}");
    let diag = session.recv();
    let diags = diag["params"]["diagnostics"].as_array().unwrap().clone();
    assert!(diags.iter().any(|d| d["code"] == "W201" && d["message"].as_str().unwrap_or("").contains("project.bogus")), "{diag}");
    session.send("{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"textDocument/hover\",\"params\":{\"textDocument\":{\"uri\":\"file:///Project.config\"},\"position\":{\"line\":2,\"character\":9}}}");
    let hover = session.recv();
    let value = hover["result"]["contents"]["value"].as_str().unwrap_or("");
    assert!(value.contains("Package name"), "{hover}");
    session.send("{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"textDocument/completion\",\"params\":{\"textDocument\":{\"uri\":\"file:///Project.config\"},\"position\":{\"line\":4,\"character\":0}}}");
    let complete = session.recv();
    let labels: Vec<&str> = complete["result"]["items"].as_array().unwrap().iter().filter_map(|i| i["label"].as_str()).collect();
    assert!(labels.contains(&"entry"), "{complete}");
    session.send("{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"textDocument/formatting\",\"params\":{\"textDocument\":{\"uri\":\"file:///Project.config\"},\"options\":{}}}");
    let formatted = session.recv();
    let edits = formatted["result"].as_array().unwrap().clone();
    assert_eq!(edits.len(), 1, "{formatted}");
    assert!(edits[0]["newText"].as_str().unwrap_or("").contains("name: \"demo\""), "{formatted}");
    session.shutdown();
    session.stop(0);
}
