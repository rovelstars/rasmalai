use diagnostics::{line_col, JsonDiagnostic};
use std::path::{Path, PathBuf};

pub struct LintOutcome {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

fn relative_display(path: &Path) -> String {
    let text = path.to_string_lossy().into_owned();
    match std::env::current_dir() {
        Ok(cwd) => match path.strip_prefix(&cwd) {
            Ok(rel) => rel.to_string_lossy().into_owned(),
            Err(_) => text,
        },
        Err(_) => text,
    }
}

fn color_enabled() -> bool {
    std::env::var("NO_COLOR").is_err()
}

fn paint(code: &str, text: &str) -> String {
    if color_enabled() {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

fn render_finding(
    diag: &frontend::lint::LintDiagnostic,
    display: &str,
    line: usize,
    col: usize,
    src: &str,
) -> String {
    let label = paint("1;33", "warning");
    let rule = paint("1", diag.code);
    let mut out = format!("{display}:{line}:{col}: {label}[{rule}]: {}\n", diag.message);
    let lines: Vec<&str> = src.lines().collect();
    if let Some(line_text) = lines.get(line.saturating_sub(1)) {
        let width = line_text.len().to_string().len().max(2);
        out.push_str(&format!("{:>width$} |\n", "", width = width));
        out.push_str(&format!("{:>width$} | {line_text}\n", line, width = width));
        let start_col = col.saturating_sub(1);
        let mut len = diag
            .span
            .end
            .saturating_sub(diag.span.start) as usize;
        len = len.max(1).min(line_text.len().saturating_sub(start_col).max(1));
        out.push_str(&format!(
            "{:>width$} | {:<pad$}{}\n",
            "",
            "",
            "^".repeat(len),
            width = width,
            pad = start_col
        ));
    }
    if let Some(hint) = &diag.fix_hint {
        out.push_str(&format!("hint: {hint}\n"));
    }
    out
}

fn package_root_for(entry: &Path) -> PathBuf {
    let mut dir = entry.parent().map(|p| p.to_path_buf());
    while let Some(d) = dir {
        if d.join("Project.config").is_file() {
            return d;
        }
        dir = d.parent().map(|p| p.to_path_buf());
    }
    entry
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn resolve_roots(paths: &[PathBuf], package: Option<&str>) -> Result<Vec<PathBuf>, String> {
    if !paths.is_empty() {
        return Ok(paths.to_vec());
    }
    if let Some(name) = package {
        return match crate::resolve_scope_target(None, Some(name)) {
            Ok(target) => Ok(vec![package_root_for(Path::new(&target.entry))]),
            Err(e) => Err(format!("error: {e}")),
        };
    }
    match std::env::current_dir() {
        Ok(cwd) => Ok(vec![cwd]),
        Err(e) => Err(format!("error: cannot read working directory: {e}")),
    }
}

fn rename_target(hint: &str) -> Option<String> {
    let mut parts = hint.split('`');
    parts.next()?;
    parts.next().map(|s| s.to_string())
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn replace_word_once(text: &str, from: usize, name: &str) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    let mut i = from.min(bytes.len());
    while i + name.len() <= bytes.len() {
        if &text[i..i + name.len()] == name {
            let before = i.checked_sub(1).map(|j| bytes[j] as char);
            let after = bytes.get(i + name.len()).map(|b| *b as char);
            let left_ok = before.is_none_or(|c| !is_ident_char(c));
            let right_ok = after.is_none_or(|c| !is_ident_char(c));
            if left_ok && right_ok {
                let mut out = text.to_string();
                out.replace_range(i..i + name.len(), &format!("_{name}"));
                return Some((out, i));
            }
        }
        i += 1;
    }
    None
}

fn apply_renames(diags: &[frontend::lint::LintDiagnostic]) -> usize {
    use std::collections::BTreeMap;
    let mut by_file: BTreeMap<PathBuf, Vec<(u32, String)>> = BTreeMap::new();
    for diag in diags {
        let (Some(path), Some(name)) = (
            diag.file.clone(),
            diag.fix_hint.as_deref().and_then(rename_target),
        ) else {
            continue;
        };
        by_file.entry(path).or_default().push((diag.span.start, name));
    }
    let mut fixed = 0usize;
    for (path, mut edits) in by_file {
        let Ok(mut text) = std::fs::read_to_string(&path) else {
            continue;
        };
        edits.sort_by_key(|(offset, _)| std::cmp::Reverse(*offset));
        let mut file_fixed = 0usize;
        for (offset, name) in edits {
            if let Some((next, _)) = replace_word_once(&text, offset as usize, &name) {
                text = next;
                file_fixed += 1;
            }
        }
        if file_fixed > 0 {
            let _ = std::fs::write(&path, text);
            fixed += file_fixed;
        }
    }
    fixed
}

pub fn run_lint(
    paths: &[PathBuf],
    package: Option<&str>,
    sarif: bool,
    json: bool,
    deny_warnings: bool,
    fix: bool,
) -> LintOutcome {
    let mut stdout = String::new();
    let mut stderr = String::new();
    let fail = |stderr: String| LintOutcome {
        code: 1,
        stdout: String::new(),
        stderr,
    };
    let roots = match resolve_roots(paths, package) {
        Ok(roots) => roots,
        Err(message) => return fail(format!("{message}\n")),
    };
    let mut diags = Vec::new();
    for root in &roots {
        if !root.exists() {
            return fail(format!("path not found: {}\n", root.display()));
        }
        diags.extend(frontend::lint::lint_package(root));
    }
    if fix {
        let fixed = apply_renames(&diags);
        if fixed > 0 {
            stderr.push_str(&format!("Fixed {fixed} findings\n"));
            diags.clear();
            for root in &roots {
                diags.extend(frontend::lint::lint_package(root));
            }
        }
    }
    if sarif {
        let mut results = Vec::new();
        for diag in &diags {
            let path = diag.file.clone().unwrap_or_else(|| PathBuf::from("<input>"));
            let src = std::fs::read_to_string(&path).unwrap_or_default();
            let (line, col) = line_col(&src, diag.span.start);
            results.push(frontend::sarif::SarifResult {
                file: relative_display(&path),
                line,
                col,
                rule: diag.code,
                message: diag.message.clone(),
            });
        }
        stdout.push_str(&frontend::sarif::sarif_report(&results));
        stdout.push('\n');
    } else if json {
        let mut items = Vec::new();
        for diag in &diags {
            let path = diag.file.clone().unwrap_or_else(|| PathBuf::from("<input>"));
            let src = std::fs::read_to_string(&path).unwrap_or_default();
            let (line, col) = line_col(&src, diag.span.start);
            items.push(JsonDiagnostic {
                file: relative_display(&path),
                line,
                col,
                code: diag.code.to_string(),
                severity: "warning".to_string(),
                message: diag.message.clone(),
            });
        }
        stdout.push_str(&diagnostics::diagnostics_to_json(&items));
        stdout.push('\n');
    } else if diags.is_empty() {
        stderr.push_str("No lint issues found.\n");
    } else {
        for diag in &diags {
            let path = diag.file.clone().unwrap_or_else(|| PathBuf::from("<input>"));
            let src = std::fs::read_to_string(&path).unwrap_or_default();
            let (line, col) = line_col(&src, diag.span.start);
            stderr.push_str(&render_finding(diag, &relative_display(&path), line, col, &src));
        }
        let fixable = diags.iter().filter(|d| d.fix_hint.is_some()).count();
        stderr.push_str(&format!(
            "Found {} warnings ({fixable} auto-fixable)\n",
            diags.len()
        ));
    }
    let failed = deny_warnings && !diags.is_empty();
    LintOutcome {
        code: i32::from(failed),
        stdout,
        stderr,
    }
}
