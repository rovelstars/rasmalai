use crate::lsp::json::{parse_json, JVal};
use diagnostics::{escape_json, line_col};
use std::collections::HashMap;
use std::io::{BufRead, Write};

pub enum RpcId {
    Num(String),
    Str(String),
}

impl RpcId {
    fn render(&self) -> String {
        match self {
            RpcId::Num(raw) => raw.clone(),
            RpcId::Str(s) => format!("\"{}\"", escape_json(s)),
        }
    }
}

fn read_message(reader: &mut impl BufRead) -> Option<String> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => return None,
            Ok(_) => {}
            Err(_) => return None,
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some(value) = trimmed.strip_prefix("Content-Length:") {
            content_length = value.trim().parse::<usize>().ok();
        }
    }
    let length = content_length?;
    let mut buf = vec![0u8; length];
    reader.read_exact(&mut buf).ok()?;
    String::from_utf8(buf).ok()
}

fn send(text: &str) {
    print!("Content-Length: {}\r\n\r\n{text}", text.len());
    let _ = std::io::stdout().flush();
}

fn send_response(id: &RpcId, result_json: &str) {
    send(&format!("{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{result_json}}}", id.render()));
}

fn send_error(id: &RpcId, code: i64, message: &str) {
    send(&format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{},\"error\":{{\"code\":{code},\"message\":\"{}\"}}}}",
        id.render(),
        escape_json(message)
    ));
}

fn send_notification(method: &str, params_json: &str) {
    send(&format!("{{\"jsonrpc\":\"2.0\",\"method\":\"{method}\",\"params\":{params_json}}}"));
}

fn rpc_id(value: &JVal) -> Option<RpcId> {
    match value.get("id") {
        Some(JVal::Num(raw)) => Some(RpcId::Num(raw.clone())),
        Some(JVal::Str(s)) => Some(RpcId::Str(s.clone())),
        _ => None,
    }
}

fn lsp_range(src: &str, start: u32, end: u32) -> String {
    let (sl, sc) = line_col(src, start);
    let (el, ec) = line_col(src, end);
    format!(
        "{{\"start\":{{\"line\":{},\"character\":{}}},\"end\":{{\"line\":{},\"character\":{}}}}}",
        sl.saturating_sub(1),
        sc.saturating_sub(1),
        el.saturating_sub(1),
        ec.saturating_sub(1),
    )
}

fn is_config_uri(uri: &str) -> bool {
    let base = uri.rsplit('/').next().unwrap_or(uri);
    base == frontend::project::MANIFEST_FILE || base == frontend::deplock::LOCK_FILE
}

fn is_manifest_uri(uri: &str) -> bool {
    uri.rsplit('/').next().unwrap_or(uri) == frontend::project::MANIFEST_FILE
}

fn uri_dir(uri: &str) -> Option<std::path::PathBuf> {
    let path = uri.strip_prefix("file://").unwrap_or(uri);
    std::path::Path::new(path).parent().map(|p| p.to_path_buf())
}

fn line_range_json(line_0: usize) -> String {
    format!(
        "{{\"start\":{{\"line\":{line_0},\"character\":0}},\"end\":{{\"line\":{line_0},\"character\":10000}}}}"
    )
}

fn analyze_config(uri: &str, src: &str) -> String {
    let root = uri_dir(uri);
    let mut items: Vec<String> = Vec::new();
    if is_manifest_uri(uri) {
        for issue in
            frontend::project::validate_manifest_text(src, root.as_deref())
        {
            items.push(format!(
                "{{\"range\":{},\"severity\":{},\"code\":\"{}\",\"source\":\"rasmalai\",\"message\":\"{}\"}}",
                line_range_json(issue.line),
                if issue.error { 1 } else { 2 },
                issue.code.as_str(),
                escape_json(&issue.message),
            ));
        }
        if let Some(root) = root
            && let Ok(lock_text) = std::fs::read_to_string(root.join(frontend::deplock::LOCK_FILE))
            && let (Ok(lock), Ok(Some(config))) = (
                frontend::deplock::ProjectDepLock::parse(&lock_text),
                frontend::project::ProjectConfig::load_from_dir(&root),
            )
            && let Err(diag) = frontend::deplock::verify_lock(&root, &config, &lock)
        {
            items.push(format!(
                "{{\"range\":{},\"severity\":1,\"code\":\"{}\",\"source\":\"rasmalai\",\"message\":\"Project.deplock: {}\"}}",
                line_range_json(0),
                diag.code.as_str(),
                escape_json(&diag.message),
            ));
        }
    } else if let Err(msg) = frontend::deplock::ProjectDepLock::parse(src) {
        items.push(format!(
            "{{\"range\":{},\"severity\":1,\"code\":\"E108\",\"source\":\"rasmalai\",\"message\":\"{}\"}}",
            line_range_json(0),
            escape_json(&msg),
        ));
    }
    format!(
        "{{\"uri\":\"{}\",\"diagnostics\":[{}]}}",
        escape_json(uri),
        items.join(",")
    )
}

fn config_section_at(src: &str, line: usize) -> String {
    let lines: Vec<&str> = src.lines().collect();
    let mut stack: Vec<String> = Vec::new();
    for l in lines.iter().take(line + 1) {
        let t = l.trim();
        if let Some((key, _)) = t.split_once(':') {
            let key = key.trim().trim_matches('"').trim();
            if !key.is_empty()
                && key.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-')
                && t.ends_with('{')
            {
                stack.push(key.to_string());
                continue;
            }
        }
        let opens = t.chars().filter(|&c| c == '{').count();
        let closes = t.chars().filter(|&c| c == '}').count();
        for _ in 0..closes.saturating_sub(opens) {
            stack.pop();
        }
    }
    stack.last().cloned().unwrap_or_default()
}

fn config_word_at(line_text: &str, character: usize) -> String {
    let chars: Vec<char> = line_text.chars().collect();
    let mut col = character.min(chars.len());
    while col > 0 && (chars[col - 1].is_alphanumeric() || chars[col - 1] == '_' || chars[col - 1] == '-') {
        col -= 1;
    }
    let start = col;
    while col < chars.len() && (chars[col].is_alphanumeric() || chars[col] == '_' || chars[col] == '-') {
        col += 1;
    }
    chars[start..col].iter().collect()
}

fn analyze(uri: &str, src: &str) -> String {
    let (warnings, errors) = frontend::check::check_source_all(src);
    let mut items: Vec<String> = Vec::new();
    for diag in &errors {
        let (start, end) = match diag.span {
            Some(span) => (span.start, span.end),
            None => (0, 0),
        };
        items.push(format!(
            "{{\"range\":{},\"severity\":1,\"code\":\"{}\",\"source\":\"rasmalai\",\"message\":\"{}\"}}",
            lsp_range(src, start, end),
            diag.code.as_str(),
            escape_json(&diag.message),
        ));
    }
    for diag in &warnings {
        let (start, end) = match diag.span {
            Some(span) => (span.start, span.end),
            None => (0, 0),
        };
        items.push(format!(
            "{{\"range\":{},\"severity\":2,\"code\":\"{}\",\"source\":\"rasmalai\",\"message\":\"{}\"}}",
            lsp_range(src, start, end),
            diag.code.as_str(),
            escape_json(&diag.message),
        ));
    }
    for lint in frontend::lint::lint_source(src) {
        items.push(format!(
            "{{\"range\":{},\"severity\":2,\"code\":\"{}\",\"source\":\"rasmalai\",\"message\":\"{}\"}}",
            lsp_range(src, lint.span.start, lint.span.end),
            lint.code,
            escape_json(&lint.message),
        ));
    }
    format!(
        "{{\"uri\":\"{}\",\"diagnostics\":[{}]}}",
        escape_json(uri),
        items.join(",")
    )
}

fn publish(uri: &str, src: &str) {
    if is_config_uri(uri) {
        send_notification("textDocument/publishDiagnostics", &analyze_config(uri, src));
    } else if uri.ends_with(".rnx") {
        send_notification("textDocument/publishDiagnostics", &analyze(uri, src));
    } else {
        publish_empty(uri);
    }
}

fn publish_empty(uri: &str) {
    send_notification(
        "textDocument/publishDiagnostics",
        &format!("{{\"uri\":\"{}\",\"diagnostics\":[]}}", escape_json(uri)),
    );
}

fn config_hover(src: &str, line: usize, character: usize) -> Option<String> {
    let line_text = src.lines().nth(line)?;
    let word = config_word_at(line_text, character);
    if word.is_empty() {
        return None;
    }
    let section = config_section_at(src, line);
    if line_text.trim_start().starts_with('[') {
        let name = word.trim_matches(['[', ']']);
        let doc = frontend::project::manifest_section_doc(name)?;
        return Some(format!("```rnx\n{name}\n```\n\n{doc}"));
    }
    let doc = frontend::project::manifest_field_doc(&section, &word)?;
    Some(format!("```rnx\n{word}\n```\n\n{doc}"))
}

fn config_value_completions(
    root: Option<&std::path::Path>,
    section: &str,
    key: &str,
) -> Vec<frontend::lsp_completion::CompletionItem> {
    let mut items = Vec::new();
    let item = |label: &str, detail: &str| frontend::lsp_completion::CompletionItem {
        label: label.to_string(),
        kind: 12,
        detail: detail.to_string(),
        insert_text: label.to_string(),
    };
    match (section, key) {
        ("project", "version") => {
            items.push(item("\"0.1.0\"", "SemVer major.minor.patch"));
        }
        ("project", "entry") => {
            if let Some(root) = root {
                let mut found = Vec::new();
                collect_rnx_files(root, root, &mut found, 3);
                found.sort();
                for rel in found.iter().take(20) {
                    items.push(item(
                        &format!("\"{rel}\""),
                        "entry file relative to the project root",
                    ));
                }
            }
            if items.is_empty() {
                items.push(item("\"src/main.rnx\"", "default entry file"));
            }
        }
        ("dependencies", _) => {
            items.push(item(
                "{ path = \"\" }",
                "local path dependency with its own Project.config",
            ));
            items.push(item(
                "{ git = \"\", rev = \"\" }",
                "pinned git dependency (tag or commit, never a branch)",
            ));
        }
        ("workspace", "members") => {
            if let Some(root) = root
                && let Ok(entries) = std::fs::read_dir(root)
            {
                let mut dirs: Vec<String> = entries
                    .flatten()
                    .filter(|e| {
                        e.path().is_dir() && e.path().join(frontend::project::MANIFEST_FILE).is_file()
                    })
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect();
                dirs.sort();
                for d in dirs.iter().take(20) {
                    items.push(item(
                        &format!("\"{d}\""),
                        "workspace member package",
                    ));
                }
            }
        }
        _ => {}
    }
    items
}

fn collect_rnx_files(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<String>, depth: usize) {
    if depth == 0 {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == "target" || name == ".git" || name == ".rnx" {
                continue;
            }
            collect_rnx_files(root, &path, out, depth - 1);
        } else if path.extension().is_some_and(|e| e == "rnx") {
            let rel = path
                .strip_prefix(root)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| path.to_string_lossy().replace('\\', "/"));
            out.push(rel);
        }
    }
}

fn config_key_at(line_text: &str) -> Option<String> {
    let before_eq = line_text.split(':').next()?;
    let key = before_eq.trim().trim_matches('"');
    if key.is_empty() || key.contains(' ') || key.contains('[') || key.contains('{') {
        return None;
    }
    if !key.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
        return None;
    }
    Some(key.to_string())
}

fn config_completions(
    src: &str,
    line: usize,
    line_text: &str,
    root: Option<std::path::PathBuf>,
) -> Option<String> {
    let t = line_text.trim();
    let mut items = Vec::new();
    if t.starts_with('[') {
        for section in frontend::project::manifest_sections() {
            items.push(frontend::lsp_completion::CompletionItem {
                label: format!("[{section}]"),
                kind: 14,
                detail: "section".to_string(),
                insert_text: format!("[{section}]"),
            });
        }
    } else {
        let section = config_section_at(src, line);
        if line_text.contains(':') {
            if let Some(key) = config_key_at(line_text) {
                let values = config_value_completions(root.as_deref(), &section, &key);
                if !values.is_empty() {
                    return Some(frontend::lsp_completion::completions_json(&values));
                }
            }
        }
        for key in frontend::project::manifest_keys(&section) {
            let detail = frontend::project::manifest_field_doc(&section, key)
                .unwrap_or("")
                .to_string();
            items.push(frontend::lsp_completion::CompletionItem {
                label: (*key).to_string(),
                kind: 10,
                detail,
                insert_text: format!("{key}: "),
            });
        }
        if items.is_empty() {
            return None;
        }
    }
    Some(frontend::lsp_completion::completions_json(&items))
}

fn request_doc<'a>(
    message: &JVal,
    docs: &'a HashMap<String, String>,
) -> Option<(String, &'a str)> {
    let uri = message
        .get("params")
        .and_then(|p| p.get("textDocument"))
        .and_then(|d| d.get("uri"))
        .and_then(|u| u.as_str())?;
    let src = docs.get(uri)?;
    Some((uri.to_string(), src.as_str()))
}

fn request_position(message: &JVal) -> Option<(usize, usize)> {
    let position = message.get("params").and_then(|p| p.get("position"))?;
    let line = match position.get("line") {
        Some(JVal::Num(raw)) => raw.parse::<usize>().ok()?,
        _ => return None,
    };
    let character = match position.get("character") {
        Some(JVal::Num(raw)) => raw.parse::<usize>().ok()?,
        _ => return None,
    };
    Some((line, character))
}

fn request_offset(message: &JVal, src: &str) -> Option<usize> {
    let (line, character) = request_position(message)?;
    frontend::lsp_nav::position_to_offset(src, line, character)
}

fn lookup_symbol(
    message: &JVal,
    docs: &HashMap<String, String>,
) -> Option<frontend::lsp_nav::SymbolTarget> {
    let (_, src) = request_doc(message, docs)?;
    let offset = request_offset(message, src)?;
    let module = frontend::parser::Parser::parse_module(src).ok()?;
    frontend::lsp_nav::find_symbol_at(&module, src, offset)
}

fn hover_target(message: &JVal, docs: &HashMap<String, String>) -> Option<(String, String)> {
    lookup_symbol(message, docs).map(|target| (target.signature, target.docs))
}

fn definition_target(message: &JVal, docs: &HashMap<String, String>) -> Option<(String, u32, u32)> {
    let (uri, _) = request_doc(message, docs)?;
    let target = lookup_symbol(message, docs)?;
    Some((uri, target.span.start, target.span.end))
}

fn parse_for_nav(src: &str, line: usize) -> Option<(frontend::ast::Module, String)> {
    if let Ok(module) = frontend::parser::Parser::parse_module(src) {
        return Some((module, src.to_string()));
    }
    let mut lines: Vec<&str> = src.split('\n').collect();
    if line >= lines.len() {
        return None;
    }
    lines[line] = "";
    let blanked = lines.join("\n");
    frontend::parser::Parser::parse_module(&blanked)
        .ok()
        .map(|module| (module, blanked))
}

fn formatting_edit(message: &JVal, docs: &HashMap<String, String>) -> String {
    let (uri, src) = match request_doc(message, docs) {
        Some(doc) => doc,
        None => return "null".to_string(),
    };
    let formatted = if is_manifest_uri(&uri) {
        match frontend::project::format_manifest_text(src) {
            Ok(formatted) => formatted,
            Err(_) => return "null".to_string(),
        }
    } else if is_config_uri(&uri) {
        match frontend::deplock::ProjectDepLock::parse(src) {
            Ok(lock) => lock.to_rnx(),
            Err(_) => return "null".to_string(),
        }
    } else {
        match frontend::fmt::format_source(src) {
            Ok(formatted) => formatted,
            Err(_) => return "null".to_string(),
        }
    };
    if formatted == src {
        return "[]".to_string();
    }
    let (last_line, last_col) = if src.ends_with('\n') {
        (src.lines().count(), 0)
    } else {
        let count = src.lines().count();
        let col = src.lines().last().map(|l| l.chars().count()).unwrap_or(0);
        (count.saturating_sub(1), col)
    };
    format!(
        "[{{\"range\":{{\"start\":{{\"line\":0,\"character\":0}},\"end\":{{\"line\":{last_line},\"character\":{last_col}}}}},\"newText\":\"{}\"}}]",
        escape_json(&formatted)
    )
}

fn semantic_tokens_json(src: &str) -> String {
    let module = match frontend::parser::Parser::parse_module(src) {
        Ok(module) => module,
        Err(_) => return "{\"data\":[]}".to_string(),
    };
    let mut data: Vec<u32> = Vec::new();
    let mut prev_line = 0u32;
    let mut prev_col = 0u32;
    for span in frontend::semantic_tokens::parameter_spans(&module) {
        let (line, col) = frontend::lsp_nav::offset_to_position(src, span.start as usize);
        let len = src
            .get(span.start as usize..span.end as usize)
            .map(|s| s.chars().count())
            .unwrap_or(0);
        let (line, col, len) = (line as u32, col as u32, len as u32);
        data.push(line - prev_line);
        data.push(if line == prev_line { col - prev_col } else { col });
        data.push(len);
        data.push(0);
        data.push(0);
        prev_line = line;
        prev_col = col;
    }
    format!(
        "{{\"data\":[{}]}}",
        data.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(",")
    )
}

fn e105_fix(src: &str, start: usize, end: usize) -> Option<(String, usize, usize, String)> {
    let bytes = src.as_bytes();
    if end > bytes.len() || start > end {
        return None;
    }
    match src.get(start..end) {
        Some("fn") => {
            let mut e = end;
            while e < bytes.len() && (bytes[e] == b' ' || bytes[e] == b'\t') {
                e += 1;
            }
            Some(("Convert to arrow lambda".to_string(), start, e, String::new()))
        }
        Some("=>") => {
            let line_start = src.get(..start)?.rfind('\n').map(|i| i + 1).unwrap_or(0);
            let line_end = src.get(start..)?.find('\n').map(|i| start + i).unwrap_or(src.len());
            let line = src.get(line_start..line_end)?;
            let indent: String = line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
            let after = src.get(start + 2..line_end)?;
            let mut tail = after.trim_end();
            tail = match tail.strip_suffix(';') {
                Some(t) => t,
                None => tail,
            };
            if tail.contains(';') {
                return None;
            }
            let expr = tail.trim();
            if expr.is_empty() || expr.contains('\n') {
                return None;
            }
            let replace_end = line_end - (after.len() - after.trim_end().len());
            let new_text = format!("{{\n{indent}    return {expr};\n{indent}}}");
            Some(("Convert to block body".to_string(), start, replace_end, new_text))
        }
        _ => None,
    }
}

fn code_actions_json(uri: &str, src: &str, diags: &JVal) -> String {
    let mut actions = Vec::new();
    if let JVal::Arr(items) = diags {
        for diag in items {
            if diag.get("code").and_then(|c| c.as_str()) != Some("E105") {
                continue;
            }
            let pos = |key: &str| {
                let obj = diag.get("range")?.get(key)?;
                let line = match obj.get("line") {
                    Some(JVal::Num(raw)) => raw.parse::<usize>().ok()?,
                    _ => return None,
                };
                let character = match obj.get("character") {
                    Some(JVal::Num(raw)) => raw.parse::<usize>().ok()?,
                    _ => return None,
                };
                Some((line, character))
            };
            let ((sl, sc), (el, ec)) = match (pos("start"), pos("end")) {
                (Some(a), Some(b)) => (a, b),
                _ => continue,
            };
            let (start, end) = match (
                frontend::lsp_nav::position_to_offset(src, sl, sc),
                frontend::lsp_nav::position_to_offset(src, el, ec),
            ) {
                (Some(a), Some(b)) => (a, b),
                _ => continue,
            };
            if let Some((title, es, ee, text)) = e105_fix(src, start, end) {
                actions.push(format!(
                    "{{\"title\":\"{}\",\"kind\":\"quickfix\",\"edit\":{{\"changes\":{{\"{}\":[{{\"range\":{},\"newText\":\"{}\"}}]}}}}}}",
                    escape_json(&title),
                    escape_json(uri),
                    lsp_range(src, es as u32, ee as u32),
                    escape_json(&text),
                ));
            }
        }
    }
    format!("[{}]", actions.join(","))
}

pub fn run_lsp() -> i32 {
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let mut docs: HashMap<String, String> = HashMap::new();
    let mut shutdown = false;
    while let Some(body) = read_message(&mut reader) {
        let message = match parse_json(&body) {
            Ok(message) => message,
            Err(e) => {
                eprintln!("lsp: dropping malformed frame: {e}");
                continue;
            }
        };
        let method = message.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let id = rpc_id(&message);
        match method {
            "initialize" => {
                if let Some(id) = id {
                    send_response(
                        &id,
                        "{\"capabilities\":{\"textDocumentSync\":1,\"hoverProvider\":true,\"definitionProvider\":true,\"documentFormattingProvider\":true,\"documentSymbolProvider\":true,\"codeActionProvider\":true,\"semanticTokensProvider\":{\"legend\":{\"tokenTypes\":[\"parameter\"],\"tokenModifiers\":[]},\"full\":true},\"completionProvider\":{\"triggerCharacters\":[\".\"]}},\"serverInfo\":{\"name\":\"rasmalai-lsp\",\"version\":\"1.0.0\"}}",
                    );
                }
            }
            "initialized" => {}
            "shutdown" => {
                shutdown = true;
                if let Some(id) = id {
                    send_response(&id, "null");
                }
            }
            "exit" => break,
            "textDocument/didOpen" => {
                let params = message.get("params");
                let doc = params.and_then(|p| p.get("textDocument"));
                let uri = doc.and_then(|d| d.get("uri")).and_then(|u| u.as_str());
                let text = doc.and_then(|d| d.get("text")).and_then(|t| t.as_str());
                match (uri, text) {
                    (Some(uri), Some(text)) => {
                        docs.insert(uri.to_string(), text.to_string());
                        publish(uri, text);
                    }
                    _ => eprintln!("lsp: didOpen missing uri or text"),
                }
            }
            "textDocument/didChange" => {
                let params = message.get("params");
                let doc = params.and_then(|p| p.get("textDocument"));
                let uri = doc.and_then(|d| d.get("uri")).and_then(|u| u.as_str());
                let changes = params.and_then(|p| p.get("contentChanges"));
                let text = match changes {
                    Some(JVal::Arr(items)) => items
                        .last()
                        .and_then(|c| c.get("text"))
                        .and_then(|t| t.as_str()),
                    _ => None,
                };
                match (uri, text) {
                    (Some(uri), Some(text)) => {
                        docs.insert(uri.to_string(), text.to_string());
                        publish(uri, text);
                    }
                    _ => eprintln!("lsp: didChange missing uri or text"),
                }
            }
            "textDocument/didClose" => {
                let uri = message
                    .get("params")
                    .and_then(|p| p.get("textDocument"))
                    .and_then(|d| d.get("uri"))
                    .and_then(|u| u.as_str());
                match uri {
                    Some(uri) => {
                        docs.remove(uri);
                        publish_empty(uri);
                    }
                    None => eprintln!("lsp: didClose missing uri"),
                }
            }
            "textDocument/hover" => {
                if let Some(id) = id {
                    let result = (|| {
                        let (uri, src) = request_doc(&message, &docs)?;
                        if is_config_uri(&uri) {
                            let (line, character) = request_position(&message)?;
                            return config_hover(src, line, character);
                        }
                        hover_target(&message, &docs).map(|(signature, docs_text)| {
                            frontend::doc::render_hover_markdown(&signature, &docs_text)
                        })
                    })();
                    let result = result.map(|value| {
                        format!(
                            "{{\"contents\":{{\"kind\":\"markdown\",\"value\":\"{}\"}}}}",
                            escape_json(&value)
                        )
                    });
                    send_response(&id, result.as_deref().unwrap_or("null"));
                }
            }
            "textDocument/definition" => {
                if let Some(id) = id {
                    let result = definition_target(&message, &docs).map(|(uri, start, end)| {
                        let (sl, sc) = frontend::lsp_nav::offset_to_position(&docs[&uri], start as usize);
                        let (el, ec) = frontend::lsp_nav::offset_to_position(&docs[&uri], end as usize);
                        format!(
                            "{{\"uri\":\"{}\",\"range\":{{\"start\":{{\"line\":{sl},\"character\":{sc}}},\"end\":{{\"line\":{el},\"character\":{ec}}}}}}}",
                            escape_json(&uri)
                        )
                    });
                    send_response(&id, result.as_deref().unwrap_or("null"));
                }
            }
            "textDocument/formatting" => {
                if let Some(id) = id {
                    send_response(&id, &formatting_edit(&message, &docs));
                }
            }
            "textDocument/documentSymbol" => {
                if let Some(id) = id {
                    let result = (|| {
                        let (_, src) = request_doc(&message, &docs)?;
                        let module = frontend::parser::Parser::parse_module(src).ok()?;
                        Some(frontend::lsp_symbols::symbols_json(&module, src))
                    })();
                    send_response(&id, result.as_deref().unwrap_or("[]"));
                }
            }
            "textDocument/completion" => {
                if let Some(id) = id {
                    let result = (|| {
                        let (uri, src) = request_doc(&message, &docs)?;
                        let (line, character) = request_position(&message)?;
                        if is_config_uri(&uri) {
                            let line_text = src.split('\n').nth(line).unwrap_or("");
                            let _ = character;
                            return config_completions(src, line, line_text, uri_dir(&uri));
                        }
                        let (module, effective) = parse_for_nav(src, line)?;
                        let line_text = src.split('\n').nth(line).unwrap_or("");
                        Some(frontend::lsp_completion::completions_json(
                            &frontend::lsp_completion::get_completions(&module, &effective, line_text, line, character),
                        ))
                    })();
                    send_response(&id, result.as_deref().unwrap_or("null"));
                }
            }
            "textDocument/codeAction" => {
                if let Some(id) = id {
                    let result = (|| {
                        let (uri, src) = request_doc(&message, &docs)?;
                        let diags = message.get("params")?.get("context")?.get("diagnostics")?;
                        Some(code_actions_json(&uri, src, diags))
                    })();
                    send_response(&id, result.as_deref().unwrap_or("null"));
                }
            }
            "textDocument/semanticTokens/full" => {
                if let Some(id) = id {
                    let result = (|| {
                        let (_, src) = request_doc(&message, &docs)?;
                        Some(semantic_tokens_json(src))
                    })();
                    send_response(&id, result.as_deref().unwrap_or("null"));
                }
            }
            _ => {
                if let Some(id) = id {
                    send_error(&id, -32601, "Method not found");
                }
            }
        }
    }
    i32::from(!shutdown)
}
