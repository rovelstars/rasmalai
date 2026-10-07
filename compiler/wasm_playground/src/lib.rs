use diagnostics::theme::AuraTheme;
use diagnostics::tree_hook::{TreeHook, render};
use diagnostics::{line_col, Diagnostic};
use frontend::ast as A;
use frontend::token::TokenKind;
use runtime::machine::{ExecError, Machine};
use wasm_bindgen::prelude::*;

fn prepare(source: &str) -> Result<A::Module, Diagnostic> {
    frontend::modules::ModuleGraph::from_source(source)
}
fn check_merged(source: &str) -> Result<(Vec<Diagnostic>, Vec<Diagnostic>), Diagnostic> {
    let mut module = prepare(source)?;
    let mut diags = frontend::desugar::desugar(&mut module);
    diags.extend(frontend::semantic::check(&module));
    let mut warnings = Vec::new();
    let mut errors = Vec::new();
    for d in diags {
        if d.code.is_warning() {
            warnings.push(d);
        } else {
            errors.push(d);
        }
    }
    Ok((warnings, errors))
}

fn role_of(kind: &TokenKind) -> &'static str {
    match kind {
        TokenKind::Fn
        | TokenKind::Let
        | TokenKind::Const
        | TokenKind::Return
        | TokenKind::Public
        | TokenKind::Private
        | TokenKind::Defer
        | TokenKind::Unsafe
        | TokenKind::Static
        | TokenKind::If
        | TokenKind::Else
        | TokenKind::For
        | TokenKind::While
        | TokenKind::In
        | TokenKind::Break
        | TokenKind::Continue
        | TokenKind::Switch
        | TokenKind::Case
        | TokenKind::Default
        | TokenKind::Throw
        | TokenKind::Throws
        | TokenKind::Try
        | TokenKind::Catch
        | TokenKind::Finally
        | TokenKind::Guard
        | TokenKind::Do
        | TokenKind::Fallthrough
        | TokenKind::Is
        | TokenKind::Import
        | TokenKind::From
        | TokenKind::As
        | TokenKind::Async
        | TokenKind::Await => "keyword",
        TokenKind::Class
        | TokenKind::Struct
        | TokenKind::Record
        | TokenKind::Trait
        | TokenKind::Enum
        | TokenKind::Init
        | TokenKind::Deinit
        | TokenKind::OnReload
        | TokenKind::Extends
        | TokenKind::With
        | TokenKind::Comptime
        | TokenKind::Native => "keyword",
        TokenKind::Int(_)
        | TokenKind::Float(_)
        | TokenKind::StrText(_)
        | TokenKind::StrOpen
        | TokenKind::StrClose
        | TokenKind::True
        | TokenKind::False
        | TokenKind::Null => "literal",
        TokenKind::Ident(name) if is_type_name(name) => "type",
        TokenKind::Ident(_) | TokenKind::This => "text",
        _ => "plain",
    }
}

fn is_type_name(word: &str) -> bool {
    matches!(
        word,
        "Int"
            | "Float"
            | "FastFloat"
            | "Bool"
            | "Void"
            | "String"
            | "Any"
            | "Array"
            | "Map"
            | "Set"
            | "GenRef"
            | "Vec4f"
            | "Vec4i"
            | "Vec2"
            | "Option"
            | "Result"
    )
}

fn token_text<'a>(src: &'a str, start: u32, end: u32) -> &'a str {
    let start = (start as usize).min(src.len());
    let end = (end as usize).min(src.len()).max(start);
    src.get(start..end).unwrap_or("")
}

fn diag_hook(src: &str, diag: &Diagnostic) -> String {
    let theme = AuraTheme::plain();
    let kind = if diag.code.is_warning() { "warning" } else { "error" };
    let (line, col) = diag.span.map(|s| line_col(src, s.start)).unwrap_or((1, 1));
    let lines: Vec<&str> = src.lines().collect();
    let mut context = Vec::new();
    if !lines.is_empty() {
        let lo = line.saturating_sub(2).max(1);
        let hi = (line + 1).min(lines.len());
        for n in lo..=hi {
            context.push((n, lines[n - 1].to_string()));
        }
    }
    render(
        &theme,
        &TreeHook {
            title: format!("{kind}[{}]: {}", diag.code, diag.message),
            location: Some(format!("line {line}, column {col}")),
            context,
            pointer_line: line,
            pointer_col: col.saturating_sub(1),
            got: None,
            expected: None,
            footer: diag.hint.clone(),
            footer2: None,
        },
    )
}

#[wasm_bindgen]
pub fn tokenize(source: &str) -> String {
    let toks = match frontend::lexer::lex(source) {
        Ok(toks) => toks,
        Err(e) => {
            return format!(
                "{{\"error\":\"{}\"}}",
                diagnostics::escape_json(&e.to_string())
            );
        }
    };
    let mut out = String::from("{\"tokens\":[");
    for (i, tok) in toks.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{{\"role\":\"{}\",\"text\":\"{}\",\"start\":{},\"end\":{}}}",
            role_of(&tok.kind),
            diagnostics::escape_json(token_text(source, tok.span.start, tok.span.end)),
            tok.span.start,
            tok.span.end,
        ));
    }
    out.push_str("]}");
    out
}

#[wasm_bindgen]
pub fn check(source: &str) -> String {
    let (warnings, errors) = match check_merged(source) {
        Ok(pair) => pair,
        Err(e) => return diag_hook(source, &e),
    };
    if warnings.is_empty() && errors.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for diag in errors.iter().chain(warnings.iter()) {
        out.push_str(&diag_hook(source, diag));
    }
    out
}

#[wasm_bindgen]
pub fn run(source: &str) -> String {
    let (warnings, errors) = match check_merged(source) {
        Ok(pair) => pair,
        Err(e) => return diag_hook(source, &e),
    };
    if !errors.is_empty() {
        let mut out = String::new();
        for diag in errors.iter().chain(warnings.iter()) {
            out.push_str(&diag_hook(source, diag));
        }
        return out;
    }
    let mut module = match prepare(source) {
        Ok(m) => m,
        Err(e) => return diag_hook(source, &e),
    };
    frontend::desugar::desugar(&mut module);
    let mut lowered = match lir::lower::lower(&module) {
        Ok(l) => l,
        Err(e) => return diag_hook(source, &e),
    };
    let entry = if lowered.functions.iter().any(|f| f.name == "Main") {
        "Main"
    } else {
        "main"
    };
    lir::opt::optimize_lir(&mut lowered, 1, entry);
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(lowered));
    let mut machine = Machine::new(leaked);
    let mut out = String::new();
    match machine.call(entry, Vec::new()) {
        Ok(v) => {
            for line in &machine.output {
                out.push_str(line);
                out.push('\n');
            }
            out.push_str(&format!("=> {}", v.display()));
        }
        Err(ExecError::Throw(v)) => {
            out.push_str(&format!("thrown: {}", v.display()));
        }
        Err(ExecError::Fatal(m)) => {
            out.push_str(&format!("fatal: {m}"));
        }
    }
    out
}

fn std_ok(name: &str) -> String {
    format!(
        "{{\"ok\":true,\"name\":\"{}\"}}",
        diagnostics::escape_json(name)
    )
}

fn std_fail(message: String) -> String {
    format!(
        "{{\"ok\":false,\"error\":\"{}\"}}",
        diagnostics::escape_json(&message)
    )
}

fn std_imports_of(src: &str) -> Result<Vec<String>, String> {
    let module = frontend::parser::Parser::parse_module(src).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for (spec, _) in frontend::modules::module_sources(&module.decls) {
        if frontend::modules::is_std_spec(spec) {
            let rest = frontend::modules::std_rest(spec);
            if !rest.is_empty() && !out.iter().any(|n| n == rest) {
                out.push(rest.to_string());
            }
        }
    }
    Ok(out)
}

#[wasm_bindgen]
pub fn std_pin_version() -> String {
    frontend::stdvfs::pin_version().to_string()
}

#[wasm_bindgen]
pub fn std_provide(name: &str, source: &str) -> String {
    let rest = frontend::stdvfs::normalize_rest(name.trim());
    if rest.is_empty() {
        return std_fail("empty standard library module name".to_string());
    }
    if !frontend::stdvfs::is_known_module(&rest) {
        return std_fail(format!("unknown standard library module `@std/{rest}`"));
    }
    if source.is_empty() {
        return std_fail(format!("empty source for `@std/{rest}`"));
    }
    if source.len() > frontend::stdvfs::MAX_STD_MODULE_BYTES {
        return std_fail(format!(
            "source for `@std/{rest}` exceeds {} bytes",
            frontend::stdvfs::MAX_STD_MODULE_BYTES
        ));
    }
    frontend::stdvfs::set(&rest, source.to_string());
    std_ok(&rest)
}

#[wasm_bindgen]
pub fn std_missing(source: &str) -> String {
    let mut wanted = match std_imports_of(source) {
        Ok(v) => v,
        Err(e) => {
            return format!("{{\"error\":\"{}\"}}", diagnostics::escape_json(&e));
        }
    };
    if !wanted.iter().any(|n| n == "prelude") {
        wanted.push("prelude".to_string());
    }
    let mut queue = wanted;
    let mut seen: Vec<String> = Vec::new();
    while let Some(name) = queue.pop() {
        if seen.iter().any(|n| n == &name) {
            continue;
        }
        seen.push(name.clone());
        if !frontend::stdvfs::is_known_module(&name) {
            continue;
        }
        let Some(src) = frontend::stdvfs::get(&name) else {
            continue;
        };
        let imports = match std_imports_of(&src) {
            Ok(v) => v,
            Err(e) => {
                return format!("{{\"error\":\"{}\"}}", diagnostics::escape_json(&e));
            }
        };
        for dep in imports {
            if !seen.iter().any(|n| n == &dep) && !queue.iter().any(|n| n == &dep) {
                queue.push(dep);
            }
        }
    }
    seen.sort();
    let mut missing = Vec::new();
    for name in &seen {
        if !frontend::stdvfs::contains(name) {
            missing.push(name.clone());
        }
    }
    let mut out = String::from("{\"missing\":[");
    for (i, name) in missing.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!("\"{}\"", diagnostics::escape_json(name)));
    }
    out.push_str("]}");
    out
}

// ---------- IDE assists: editor tooling plus multi-file projects ----------
// Append-only surface for the playground IDE. Existing exports above are
// untouched. Every function below returns JSON or display text, reports
// failures as E108, and never panics.

fn ide_err(message: String) -> String {
    format!(
        "{{\"error\":\"E108: {}\"}}",
        diagnostics::escape_json(&message)
    )
}

fn ide_fail(message: String) -> String {
    format!(
        "{{\"ok\":false,\"error\":\"E108: {}\"}}",
        diagnostics::escape_json(&message)
    )
}

#[wasm_bindgen]
pub fn format_source(source: &str) -> String {
    match frontend::fmt::format_source(source) {
        Ok(out) => format!(
            "{{\"ok\":true,\"output\":\"{}\"}}",
            diagnostics::escape_json(&out)
        ),
        Err(e) => ide_fail(e),
    }
}

#[wasm_bindgen]
pub fn complete(source: &str, line: u32, character: u32) -> String {
    let module = match frontend::parser::Parser::parse_module(source) {
        Ok(m) => m,
        Err(e) => return ide_err(e.to_string()),
    };
    let line_text = source
        .split('\n')
        .nth(line as usize)
        .unwrap_or("");
    let items = frontend::lsp_completion::get_completions(
        &module,
        source,
        line_text,
        line as usize,
        character as usize,
    );
    frontend::lsp_completion::completions_json(&items)
}

#[wasm_bindgen]
pub fn hover(source: &str, line: u32, character: u32) -> String {
    let module = match frontend::parser::Parser::parse_module(source) {
        Ok(m) => m,
        Err(e) => return ide_err(e.to_string()),
    };
    let offset = match frontend::lsp_nav::position_to_offset(source, line as usize, character as usize) {
        Some(o) => o,
        None => return "{\"empty\":true}".to_string(),
    };
    match frontend::lsp_nav::find_symbol_at(&module, source, offset) {
        Some(t) => format!(
            "{{\"signature\":\"{}\",\"docs\":\"{}\",\"start\":{},\"end\":{}}}",
            diagnostics::escape_json(&t.signature),
            diagnostics::escape_json(&t.docs),
            t.span.start,
            t.span.end,
        ),
        None => "{\"empty\":true}".to_string(),
    }
}

#[wasm_bindgen]
pub fn diagnostics_json(source: &str) -> String {
    diag_list_json("main.rnx", source, &project_check_single(source))
}

fn project_check_single(source: &str) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    match check_merged(source) {
        Ok((warnings, errors)) => {
            out.extend(errors);
            out.extend(warnings);
        }
        Err(e) => out.push(e),
    }
    out
}

fn diag_list_json(file: &str, source: &str, diags: &[Diagnostic]) -> String {
    let mut out = String::from("[");
    for (i, diag) in diags.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let (line, col) = diag
            .span
            .map(|s| diagnostics::line_col(source, s.start))
            .unwrap_or((1, 1));
        let (start, end) = diag.span.map(|s| (s.start, s.end)).unwrap_or((0, 0));
        out.push_str(&format!(
            "{{\"file\":\"{}\",\"line\":{},\"col\":{},\"start\":{},\"end\":{},\"code\":\"{}\",\"severity\":\"{}\",\"message\":\"{}\"}}",
            diagnostics::escape_json(file),
            line,
            col,
            start,
            end,
            diagnostics::escape_json(diag.code.as_str()),
            if diag.code.is_warning() { "warning" } else { "error" },
            diagnostics::escape_json(&diag.message),
        ));
    }
    out.push(']');
    out
}

fn normalize_project_path(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.contains(':') || trimmed.contains('\\') {
        return None;
    }
    let stripped = trimmed.strip_prefix('/').unwrap_or(trimmed);
    if stripped.is_empty() {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    for piece in stripped.split('/') {
        match piece {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return None;
                }
            }
            name => parts.push(name),
        }
    }
    if parts.is_empty() {
        return None;
    }
    let joined = parts.join("/");
    if joined.len() > 240 {
        return None;
    }
    Some(joined)
}

fn project_files(files_json: &str) -> Result<std::collections::BTreeMap<String, String>, String> {
    let value: serde_json::Value = serde_json::from_str(files_json)
        .map_err(|e| format!("E108: project files are not valid JSON: {e}"))?;
    let obj = value
        .as_object()
        .ok_or_else(|| "E108: project files must be a JSON object of path to source".to_string())?;
    if obj.is_empty() {
        return Err("E108: project has no files".to_string());
    }
    if obj.len() > 64 {
        return Err("E108: project exceeds 64 files".to_string());
    }
    let mut out = std::collections::BTreeMap::new();
    for (key, val) in obj {
        let src = val
            .as_str()
            .ok_or_else(|| format!("E108: file `{key}` source must be a string"))?;
        if src.len() > 262_144 {
            return Err(format!("E108: file `{key}` exceeds 256 KiB"));
        }
        let norm = normalize_project_path(key)
            .ok_or_else(|| format!("E108: cannot resolve module `{key}`"))?;
        if out.contains_key(&norm) {
            return Err(format!("E108: duplicate file `{norm}`"));
        }
        out.insert(norm, src.to_string());
    }
    Ok(out)
}

fn project_display(path: &std::path::Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    text.strip_prefix("project/")
        .unwrap_or(&text)
        .to_string()
}

fn project_hook(
    files: &std::collections::BTreeMap<String, String>,
    entry_src: &str,
    diag: &Diagnostic,
) -> String {
    let theme = AuraTheme::plain();
    let kind = if diag.code.is_warning() { "warning" } else { "error" };
    let name = diag
        .file
        .as_ref()
        .map(|p| project_display(p))
        .unwrap_or_else(|| "main.rnx".to_string());
    let src = files.get(&name).map(|s| s.as_str()).unwrap_or(entry_src);
    let lookup = if name.starts_with("@std/") || name.starts_with("@std\\") {
        frontend::stdvfs::std_source(name.strip_prefix("@std/").unwrap_or(&name))
    } else {
        None
    };
    let owned;
    let src = match lookup {
        Some(text) => {
            owned = text;
            owned.as_str()
        }
        None => src,
    };
    let (line, col) = diag.span.map(|s| line_col(src, s.start)).unwrap_or((1, 1));
    let lines: Vec<&str> = src.lines().collect();
    let mut context = Vec::new();
    if !lines.is_empty() {
        let lo = line.saturating_sub(2).max(1);
        let hi = (line + 1).min(lines.len());
        for n in lo..=hi {
            context.push((n, lines[n - 1].to_string()));
        }
    }
    render(
        &theme,
        &TreeHook {
            title: format!("{kind}[{}]: {}", diag.code, diag.message),
            location: Some(format!("{name} line {line}, column {col}")),
            context,
            pointer_line: line,
            pointer_col: col.saturating_sub(1),
            got: None,
            expected: None,
            footer: diag.hint.clone(),
            footer2: None,
        },
    )
}

fn project_merged(
    files: &std::collections::BTreeMap<String, String>,
    entry: &str,
) -> Result<(frontend::ast::Module, Vec<Diagnostic>), String> {
    let entry_norm = normalize_project_path(entry)
        .ok_or_else(|| format!("E108: cannot resolve module `{entry}`"))?;
    if !files.contains_key(&entry_norm) {
        return Err(format!("E108: entry `{entry_norm}` not found in project"));
    }
    for (path, src) in files {
        frontend::modules::project_mount(
            &std::path::Path::new("project").join(path),
            src.clone(),
        );
    }
    let root = std::path::Path::new("project").join(&entry_norm);
    let entry_fallback = files.get(&entry_norm).map(|s| s.as_str()).unwrap_or("");
    let outcome: Result<(frontend::ast::Module, Vec<Diagnostic>), String> = (|| {
        let graph = frontend::modules::ModuleGraph::build_collecting(&root).map_err(|mut errs| {
            let diag = errs.remove(0);
            project_hook(files, entry_fallback, &diag)
        })?;
        let mut merged = graph
            .resolve()
            .map_err(|diag| project_hook(files, entry_fallback, &diag))?;
        let files_map = frontend::modules::merged_decl_files(&graph);
        let mut diags = merged.warnings.clone();
        diags.extend(frontend::desugar::desugar_with_files(&mut merged, &files_map));
        diags.extend(frontend::semantic::check_with_files(&merged, &files_map));
        if diags.iter().all(|d| d.code.is_warning()) {
            diags.extend(graph.isolation_errors());
        }
        Ok((merged, diags))
    })();
    frontend::modules::project_unmount_all();
    outcome
}

fn project_entry_src(
    files: &std::collections::BTreeMap<String, String>,
    entry: &str,
) -> String {
    normalize_project_path(entry)
        .and_then(|n| files.get(&n).cloned())
        .unwrap_or_default()
}

#[wasm_bindgen]
pub fn diagnostics_project(files_json: &str, entry: &str) -> String {
    let files = match project_files(files_json) {
        Ok(f) => f,
        Err(e) => {
            return format!(
                "[{{\"file\":\"\",\"line\":1,\"col\":1,\"start\":0,\"end\":0,\"code\":\"E108\",\"severity\":\"error\",\"message\":\"{}\"}}]",
                diagnostics::escape_json(&e)
            );
        }
    };
    let (_, diags) = match project_merged(&files, entry) {
        Ok(pair) => pair,
        Err(text) => {
            return format!(
                "[{{\"file\":\"\",\"line\":1,\"col\":1,\"start\":0,\"end\":0,\"code\":\"E108\",\"severity\":\"error\",\"message\":\"{}\"}}]",
                diagnostics::escape_json(&text)
            );
        }
    };
    let mut out = String::from("[");
    for (i, diag) in diags.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let name = diag
            .file
            .as_ref()
            .map(|p| project_display(p))
            .unwrap_or_default();
        let src = files.get(&name).map(|s| s.as_str()).unwrap_or("");
        let (line, col) = diag
            .span
            .map(|s| diagnostics::line_col(src, s.start))
            .unwrap_or((1, 1));
        let (start, end) = diag.span.map(|s| (s.start, s.end)).unwrap_or((0, 0));
        out.push_str(&format!(
            "{{\"file\":\"{}\",\"line\":{},\"col\":{},\"start\":{},\"end\":{},\"code\":\"{}\",\"severity\":\"{}\",\"message\":\"{}\"}}",
            diagnostics::escape_json(&name),
            line,
            col,
            start,
            end,
            diagnostics::escape_json(diag.code.as_str()),
            if diag.code.is_warning() { "warning" } else { "error" },
            diagnostics::escape_json(&diag.message),
        ));
    }
    out.push(']');
    out
}

#[wasm_bindgen]
pub fn check_project(files_json: &str, entry: &str) -> String {
    let files = match project_files(files_json) {
        Ok(f) => f,
        Err(e) => return format!("error[{e}]"),
    };
    let entry_src = project_entry_src(&files, entry);
    let (_, diags) = match project_merged(&files, entry) {
        Ok(pair) => pair,
        Err(text) => return text,
    };
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    for d in diags {
        if d.code.is_warning() {
            warnings.push(d);
        } else {
            errors.push(d);
        }
    }
    if errors.is_empty() && warnings.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for diag in errors.iter().chain(warnings.iter()) {
        out.push_str(&project_hook(&files, &entry_src, diag));
    }
    out
}

fn lower_and_run(merged: &frontend::ast::Module, entry_fn: &str) -> Result<(String, Vec<String>), Diagnostic> {
    let mut lowered = lir::lower::lower(merged)?;
    lir::opt::optimize_lir(&mut lowered, 1, entry_fn);
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(lowered));
    let mut machine = Machine::new(leaked);
    match machine.call(entry_fn, Vec::new()) {
        Ok(v) => Ok((format!("=> {}", v.display()), machine.output.clone())),
        Err(ExecError::Throw(v)) => Err(Diagnostic::new(
            diagnostics::Code::E108,
            format!("thrown: {}", v.display()),
        )),
        Err(ExecError::Fatal(m)) => Err(Diagnostic::new(
            diagnostics::Code::E108,
            format!("fatal: {m}"),
        )),
    }
}

#[wasm_bindgen]
pub fn run_project(files_json: &str, entry: &str) -> String {
    let files = match project_files(files_json) {
        Ok(f) => f,
        Err(e) => return format!("error[{e}]"),
    };
    let entry_src = project_entry_src(&files, entry);
    let (merged, diags) = match project_merged(&files, entry) {
        Ok(pair) => pair,
        Err(text) => return text,
    };
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    for d in diags {
        if d.code.is_warning() {
            warnings.push(d);
        } else {
            errors.push(d);
        }
    }
    if !errors.is_empty() {
        let mut out = String::new();
        for diag in errors.iter().chain(warnings.iter()) {
            out.push_str(&project_hook(&files, &entry_src, diag));
        }
        return out;
    }
    let lowered = match lir::lower::lower(&merged) {
        Ok(l) => l,
        Err(e) => return project_hook(&files, &entry_src, &e),
    };
    let entry_fn = if lowered.functions.iter().any(|f| f.name == "Main") {
        "Main"
    } else {
        "main"
    };
    let mut owned = lowered;
    lir::opt::optimize_lir(&mut owned, 1, entry_fn);
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(owned));
    let mut machine = Machine::new(leaked);
    let mut out = String::new();
    match machine.call(entry_fn, Vec::new()) {
        Ok(v) => {
            for line in &machine.output {
                out.push_str(line);
                out.push('\n');
            }
            out.push_str(&format!("=> {}", v.display()));
        }
        Err(ExecError::Throw(v)) => {
            out.push_str(&format!("thrown: {}", v.display()));
        }
        Err(ExecError::Fatal(m)) => {
            out.push_str(&format!("fatal: {m}"));
        }
    }
    out
}

#[wasm_bindgen]
pub fn test_project(files_json: &str, entry: &str) -> String {
    let files = match project_files(files_json) {
        Ok(f) => f,
        Err(e) => return format!("error[{e}]"),
    };
    let entry_src = project_entry_src(&files, entry);
    let (merged, diags) = match project_merged(&files, entry) {
        Ok(pair) => pair,
        Err(text) => return text,
    };
    let errors: Vec<&Diagnostic> = diags.iter().filter(|d| !d.code.is_warning()).collect();
    if !errors.is_empty() {
        let mut out = String::new();
        for diag in errors {
            out.push_str(&project_hook(&files, &entry_src, diag));
        }
        return out;
    }
    let names = match frontend::harness::collect_tests(&merged, None, false) {
        Ok(n) => n,
        Err(e) => return project_hook(&files, &entry_src, &e),
    };
    if names.is_empty() {
        return "no tests found".to_string();
    }
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut out = String::new();
    for name in &names {
        match lower_and_run(&merged, name) {
            Ok(_) => {
                passed += 1;
                out.push_str(&format!("ok {name}\n"));
            }
            Err(e) => {
                failed += 1;
                out.push_str(&format!("FAIL {name}: {e}\n"));
            }
        }
    }
    out.push_str(&format!("{passed} passed, {failed} failed"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_emits_aura_roles() {
        let json = tokenize("fn main(): Int { return 42; }");
        assert!(json.contains("\"role\":\"keyword\",\"text\":\"fn\""), "{json}");
        assert!(json.contains("\"role\":\"type\",\"text\":\"Int\""), "{json}");
        assert!(json.contains("\"role\":\"literal\",\"text\":\"42\""), "{json}");
    }

    #[test]
    fn check_empty_for_valid_source() {
        assert_eq!(check("fn main(): Int { return 1; }"), String::new());
    }

    #[test]
    fn check_renders_tree_hook_for_errors() {
        let out = check("fn main(: Int { return 1; }");
        assert!(out.contains("┌─"), "{out}");
    }

    #[test]
    fn run_executes_main_and_returns_value() {
        let out = run("fn main(): Int { return 42; }");
        assert!(out.contains("=> 42"), "{out}");
    }

    #[test]
    fn run_aborts_on_assignment_type_mismatch_without_executing() {
        let out = run("fn Main(): Int { let a = 2; a = \"hello\"; return 0; }");
        assert!(out.contains("E205"), "{out}");
        assert!(!out.contains("=>"), "{out}");
    }

    #[test]
    fn run_executes_unannotated_binding_as_null() {
        let out = run("fn Main(): Int { let a; return 0; }");
        assert!(!out.contains("E206"), "{out}");
        assert!(out.contains("=> 0"), "{out}");
    }

    #[test]
    fn run_executes_recursive_class_tree() {
        let out = run("class TreeNode { let value: Int; let left: TreeNode?; let right: TreeNode?; fn count(): Int { let total = 1; if (this.left != null) { total = total + this.left.count(); } if (this.right != null) { total = total + this.right.count(); } return total; } } fn Main(): Int { let root = new TreeNode(); root.value = 10; let l = new TreeNode(); l.value = 5; root.left = l; assert(root.count() == 2, \"count\"); print(\"wasm-tree-ok\"); return 0; }");
        assert!(out.contains("wasm-tree-ok"), "{out}");
        assert!(out.contains("=> 0"), "{out}");
    }

    #[test]
    fn run_rejects_cyclic_default_construction() {
        let out = run("class Node { let next: Node = new Node(); } fn Main(): Int { return 0; }");
        assert!(out.contains("E108"), "{out}");
        assert!(out.contains("cyclic default construction"), "{out}");
        assert!(!out.contains("=>"), "{out}");
    }

    #[test]
    fn run_executes_enum_payload_match() {
        let out = run("enum Expr { Num(Int), Add(Expr, Expr) } fn eval(e: Expr): Int { switch (e) { case .Num(n): return n; case .Add(l, r): return eval(l) + eval(r); } } fn Main(): Int { let s = Expr.Add(Expr.Num(1), Expr.Num(2)); assert(eval(s) == 3, \"eval\"); print(\"wasm-enum-ok\"); return 0; }");
        assert!(out.contains("wasm-enum-ok"), "{out}");
        assert!(out.contains("=> 0"), "{out}");
    }

    #[test]
    fn run_executes_qualified_variant_pattern() {
        let out = run("enum E { A(Int), B } fn f(e: E): Int { switch (e) { case E.A(x): return x; default: return -1; } } fn Main(): Int { print(f(E.A(5))); return 0; }");
        assert!(out.contains("5"), "{out}");
        assert!(out.contains("=> 0"), "{out}");
    }

    #[test]
    fn run_executes_pointer_read_write() {
        let out = run("import { ByteBuffer } from \"@std/bytes\"; fn Main(): Int { let buf = ByteBuffer.allocate(16); unsafe { let ptr: Pointer<Int> = Pointer.fromAddress<Int>(buf.address()); ptr.write(42); assert(ptr.read() == 42, \"rw\"); *ptr = 7; assert(*ptr == 7, \"sugar\"); print(\"wasm-ptr-ok\"); } return 0; }");
        assert!(out.contains("wasm-ptr-ok"), "{out}");
        assert!(out.contains("=> 0"), "{out}");
    }

    #[test]
    fn run_rejects_pointer_read_outside_unsafe() {
        let out = run("fn Main(): Int { let p: Pointer<Int>; unsafe { p = Pointer.fromAddress<Int>(8); } let v = p.read(); return 0; }");
        assert!(out.contains("E202"), "{out}");
        assert!(!out.contains("=>"), "{out}");
    }

    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn serial() -> std::sync::MutexGuard<'static, ()> {
        SERIAL.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn genuine(name: &str) -> String {
        match name {
            "prelude" => include_str!("../../stdlib/src/prelude.rnx").to_string(),
            "fs" => include_str!("../../stdlib/src/fs.rnx").to_string(),
            _ => "export fn placeholder(): Int { return 0; }\n".to_string(),
        }
    }

    #[test]
    fn std_pin_version_matches_frontend_package() {
        assert_eq!(std_pin_version(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn std_provide_rejects_unknown_and_empty() {
        let _serial = serial();
        frontend::stdvfs::clear();
        let bad = std_provide("nope", "export fn x(): Int { return 1; }\n");
        assert!(bad.contains("\"ok\":false"), "{bad}");
        let empty = std_provide("fs", "");
        assert!(empty.contains("\"ok\":false"), "{empty}");
        let blank = std_provide("  ", "export fn x(): Int { return 1; }\n");
        assert!(blank.contains("\"ok\":false"), "{blank}");
        frontend::stdvfs::clear();
    }

    #[test]
    fn std_missing_lists_prelude_plus_direct_imports() {
        let _serial = serial();
        frontend::stdvfs::clear();
        let out = std_missing("import { hello } from \"@std/fs\";\nfn Main(): Int { return 0; }\n");
        assert!(out.contains("\"prelude\""), "{out}");
        assert!(out.contains("\"fs\""), "{out}");
        assert!(!out.contains("\"error\""), "{out}");
        frontend::stdvfs::clear();
    }

    #[test]
    fn std_missing_reports_parse_errors() {
        let _serial = serial();
        frontend::stdvfs::clear();
        let out = std_missing("fn main(: Int { return 1; }");
        assert!(out.contains("\"error\""), "{out}");
        frontend::stdvfs::clear();
    }

    #[test]
    fn std_missing_covers_transitive_deps_once_provided() {
        let _serial = serial();
        frontend::stdvfs::clear();
        assert!(std_provide("fs", &genuine("fs")).contains("\"ok\":true"));
        let out = std_missing("import { hello } from \"@std/fs\";\nfn Main(): Int { return 0; }\n");
        assert!(!out.contains("\"fs\""), "{out}");
        assert!(out.contains("\"prelude\""), "{out}");
        for name in frontend::stdvfs::list() {
            let _ = name;
        }
        let mut pending: Vec<String> = Vec::new();
        let first = out.clone();
        for name in ["bytes", "sync", "prelude"] {
            if first.contains(&format!("\"{name}\"")) {
                pending.push(name.to_string());
            }
        }
        assert!(!pending.is_empty(), "{out}");
        for name in &pending {
            assert!(std_provide(name, &genuine(name)).contains("\"ok\":true"), "{name}");
        }
        let again = std_missing("import { hello } from \"@std/fs\";\nfn Main(): Int { return 0; }\n");
        for name in &pending {
            assert!(!again.contains(&format!("\"{name}\"")), "{again}");
        }
        frontend::stdvfs::clear();
    }

    #[test]
    fn check_uses_vfs_bytes_for_std_imports() {
        let _serial = serial();
        frontend::stdvfs::clear();
        assert!(std_provide("prelude", &genuine("prelude")).contains("\"ok\":true"));
        assert!(std_provide("fs", &genuine("fs")).contains("\"ok\":true"));
        let out = check("import { mkdir } from \"@std/fs\";\nfn Main(): Int { return 0; }\n");
        assert!(!out.contains("E108"), "{out}");
        frontend::stdvfs::clear();
    }

    #[test]
    fn format_code_round_trips() {
        let raw = "fn Main():Int{return 1;}";
        let first = format_source(raw);
        assert!(first.contains("\"ok\":true"), "{first}");
        let once = serde_json::from_str::<serde_json::Value>(&first).unwrap();
        let formatted = once.get("output").and_then(|v| v.as_str()).unwrap().to_string();
        assert!(formatted.ends_with('\n'), "{formatted:?}");
        let second = format_source(&formatted);
        let twice = serde_json::from_str::<serde_json::Value>(&second).unwrap();
        assert_eq!(
            twice.get("output").and_then(|v| v.as_str()),
            Some(formatted.as_str())
        );
    }

    #[test]
    fn format_code_reports_e108_for_bad_source() {
        let out = format_source("fn Main(): Int { let s = \"unclosed; }");
        assert!(out.contains("\"ok\":false"), "{out}");
        assert!(out.contains("E108"), "{out}");
    }

    #[test]
    fn complete_lists_keywords_for_valid_source() {
        let src = "fn Main(): Int { return 0; }\n";
        let out = complete(src, 1, 0);
        assert!(out.contains("\"items\""), "{out}");
        assert!(out.contains("\"label\":\"fn\""), "{out}");
        assert!(!out.contains("\"error\""), "{out}");
    }

    #[test]
    fn complete_reports_e108_for_bad_source() {
        let out = complete("fn Main(: Int { return 1; }", 0, 0);
        assert!(out.contains("\"error\""), "{out}");
        assert!(out.contains("E108"), "{out}");
    }

    #[test]
    fn hover_returns_signature_on_definition() {
        let src = "fn Main(): Int { return 0; }\n";
        let out = hover(src, 0, 4);
        assert!(out.contains("fn Main()"), "{out}");
        assert!(out.contains("\"start\""), "{out}");
    }

    #[test]
    fn hover_empty_outside_symbols() {
        let src = "fn Main(): Int { return 0; }\n";
        let out = hover(src, 0, 0);
        assert!(out.contains("\"empty\":true") || out.contains("\"signature\""), "{out}");
    }

    #[test]
    fn hover_reports_e108_for_bad_source() {
        let out = hover("fn Main(: Int { return 1; }", 0, 0);
        assert!(out.contains("E108"), "{out}");
    }

    #[test]
    fn diagnostics_json_empty_for_valid_source() {
        assert_eq!(diagnostics_json("fn Main(): Int { return 0; }\n"), "[]");
    }

    #[test]
    fn diagnostics_json_carries_line_col_code() {
        let out = diagnostics_json("fn Main(: Int { return 1; }");
        assert!(out.contains("\"code\""), "{out}");
        assert!(out.contains("\"line\":1"), "{out}");
        assert!(out.contains("\"severity\":\"error\""), "{out}");
    }

    fn project_pair() -> (String, &'static str) {
        let files = serde_json::json!({
            "main.rnx": "import { double } from \"./util\";\nfn Main(): Int { return double(21); }\n",
            "util.rnx": "export fn double(x: Int): Int { return x * 2; }\n",
        });
        (files.to_string(), "main.rnx")
    }

    #[test]
    fn diagnostics_project_tags_files() {
        let _serial = serial();
        let files = serde_json::json!({
            "main.rnx": "import { broken } from \"./util\";\nfn Main(): Int { return broken(); }\n",
            "util.rnx": "export fn broken(): Int { let a = 1; a = \"hello\"; return 0; }\n",
        })
        .to_string();
        let out = diagnostics_project(&files, "main.rnx");
        assert!(out.contains("\"file\":\"util.rnx\""), "{out}");
        assert!(out.contains("E205"), "{out}");
        assert!(out.contains("\"severity\":\"error\""), "{out}");
        let clean = serde_json::json!({
            "main.rnx": "import { double } from \"./util\";\nfn Main(): Int { return double(21); }\n",
            "util.rnx": "export fn double(x: Int): Int { return x * 2; }\n",
        })
        .to_string();
        assert_eq!(diagnostics_project(&clean, "main.rnx"), "[]");
    }

    #[test]
    fn check_project_resolves_relative_imports() {
        let _serial = serial();
        let (files, entry) = project_pair();
        assert_eq!(check_project(&files, entry), String::new());
    }

    #[test]
    fn run_project_executes_across_files() {
        let _serial = serial();
        let (files, entry) = project_pair();
        let out = run_project(&files, entry);
        assert!(out.contains("=> 42"), "{out}");
    }

    #[test]
    fn check_project_lists_probes_for_missing_module() {
        let _serial = serial();
        let files = serde_json::json!({
            "main.rnx": "import { x } from \"./nope\";\nfn Main(): Int { return 0; }\n",
        })
        .to_string();
        let out = check_project(&files, "main.rnx");
        assert!(out.contains("E108"), "{out}");
        assert!(out.contains("nope.rnx"), "{out}");
    }

    #[test]
    fn check_project_rejects_url_imports() {
        let _serial = serial();
        let files = serde_json::json!({
            "main.rnx": "import { x } from \"https://evil.example/x\";\nfn Main(): Int { return 0; }\n",
        })
        .to_string();
        let out = check_project(&files, "main.rnx");
        assert!(out.contains("E108"), "{out}");
    }

    #[test]
    fn check_project_rejects_escaping_entry() {
        let _serial = serial();
        let (files, _) = project_pair();
        let out = check_project(&files, "../outside.rnx");
        assert!(out.contains("E108"), "{out}");
    }

    #[test]
    fn check_project_resolves_nested_index_probe() {
        let _serial = serial();
        let files = serde_json::json!({
            "main.rnx": "import { v } from \"./lib\";\nfn Main(): Int { return v(); }\n",
            "lib/index.rnx": "export fn v(): Int { return 7; }\n",
        })
        .to_string();
        assert_eq!(check_project(&files, "main.rnx"), String::new());
        let out = run_project(&files, "main.rnx");
        assert!(out.contains("=> 7"), "{out}");
    }

    #[test]
    fn test_project_runs_test_fns() {
        let _serial = serial();
        let files = serde_json::json!({
            "main.rnx": "fn Main(): Int { return 0; }\ntest fn check_math() { assert(1 + 1 == 2, \"math\"); }\n",
        })
        .to_string();
        let out = test_project(&files, "main.rnx");
        assert!(out.contains("ok check_math"), "{out}");
        assert!(out.contains("1 passed, 0 failed"), "{out}");
    }

    #[test]
    fn test_project_reports_no_tests() {
        let _serial = serial();
        let (files, entry) = project_pair();
        assert_eq!(test_project(&files, entry), "no tests found");
    }
}
