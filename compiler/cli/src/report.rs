use diagnostics::{line_col, Code, Diagnostic, Span};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub struct Attributed {
    pub path: PathBuf,
    pub display: String,
    pub line: usize,
    pub col: usize,
}

pub fn relative_display(path: &Path) -> String {
    let text = path.to_string_lossy().into_owned();
    match std::env::current_dir() {
        Ok(cwd) => match path.strip_prefix(&cwd) {
            Ok(rel) => rel.to_string_lossy().into_owned(),
            Err(_) => text,
        },
        Err(_) => text,
    }
}

pub fn render_fix_text(
    theme: &diagnostics::theme::AuraTheme,
    fix: &str,
) -> String {
    let mut out = String::new();
    for (i, part) in fix.split('`').enumerate() {
        if part.is_empty() {
            continue;
        }
        if i % 2 == 1 {
            out.push_str(&frontend::highlight::highlight_line(theme, part));
        } else {
            out.push_str(part);
        }
    }
    out
}

fn decl_spans(decl: &frontend::ast::Spanned<frontend::ast::Decl>) -> Vec<Span> {
    let mut out = vec![decl.span];
    if let frontend::ast::Decl::Import(d) = &decl.node {
        out.push(d.source_span);
    }
    if let frontend::ast::Decl::ExportFrom(d) = &decl.node {
        out.push(d.source_span);
    }
    out
}

fn exact_span(modules: &[(&PathBuf, &frontend::ast::Module)], span: Span) -> Option<usize> {
    exact_spans(modules, span).into_iter().next()
}

fn exact_spans(modules: &[(&PathBuf, &frontend::ast::Module)], span: Span) -> Vec<usize> {
    let mut out = Vec::new();
    for (i, (_, module)) in modules.iter().enumerate() {
        for decl in &module.decls {
            for cand in decl_spans(decl) {
                if cand.start == span.start && cand.end == span.end {
                    out.push(i);
                    break;
                }
            }
        }
    }
    out
}

fn tightest_span(
    modules: &[(&PathBuf, &frontend::ast::Module)],
    offset: u32,
) -> Option<(usize, u32)> {
    let mut best: Option<(usize, u32)> = None;
    let mut tied = false;
    for (i, (_, module)) in modules.iter().enumerate() {
        for decl in &module.decls {
            for cand in decl_spans(decl) {
                if cand.start <= offset && offset <= cand.end {
                    let len = cand.end.saturating_sub(cand.start);
                    match best {
                        Some((_, best_len)) if best_len < len => {}
                        Some((_, best_len)) if best_len == len => {
                            tied = true;
                        }
                        _ => {
                            best = Some((i, len));
                            tied = false;
                        }
                    }
                }
            }
        }
    }
    match (best, tied) {
        (Some(_), true) => None,
        (best, _) => best,
    }
}

pub fn attribute(
    graph: &frontend::modules::ModuleGraph,
    sources: &[(PathBuf, String)],
    diag: &Diagnostic,
) -> Attributed {
    let span = diag.span;
    let offset = span.map(|s| s.start).unwrap_or(0);
    if let Some(tagged) = diag.file.as_ref() {
        let src = source_for(sources, tagged);
        if !src.is_empty() || sources.iter().any(|(p, _)| p == tagged) {
            let (line, col) = line_col(&src, offset);
            return Attributed {
                path: tagged.clone(),
                display: relative_display(tagged),
                line,
                col,
            };
        }
    }
    let modules: Vec<(&PathBuf, &frontend::ast::Module)> = graph
        .files
        .iter()
        .map(|f| (&f.path, &f.module))
        .collect();
    let file_idx = match span.and_then(|s| exact_span(&modules, s)) {
        Some(i) => i,
        None => match tightest_span(&modules, offset) {
            Some((i, _)) => i,
            None => sources
                .iter()
                .position(|(_, src)| (offset as usize) <= src.len())
                .unwrap_or(0),
        },
    };
    let (path, src) = match sources.get(file_idx) {
        Some(entry) => entry.clone(),
        None => {
            let root = &graph.root;
            return Attributed {
                path: root.clone(),
                display: relative_display(root),
                line: 1,
                col: 1,
            };
        }
    };
    let (line, col) = line_col(&src, offset);
    Attributed {
        path: path.clone(),
        display: relative_display(&path),
        line,
        col,
    }
}

pub fn load_sources(graph: &frontend::modules::ModuleGraph) -> Vec<(PathBuf, String)> {
    graph
        .files
        .iter()
        .map(|f| (f.path.clone(), std::fs::read_to_string(&f.path).unwrap_or_default()))
        .collect()
}

pub fn source_for(sources: &[(PathBuf, String)], path: &Path) -> String {
    sources
        .iter()
        .find(|(p, _)| p == path)
        .map(|(_, s)| s.clone())
        .unwrap_or_default()
}

pub fn render_diagnostic(
    theme: &diagnostics::theme::AuraTheme,
    diag: &Diagnostic,
    attr: &Attributed,
    src: &str,
) -> String {
    render_diagnostic_with_label(theme, diag, attr, src, None)
}

pub fn render_diagnostic_with_label(
    theme: &diagnostics::theme::AuraTheme,
    diag: &Diagnostic,
    attr: &Attributed,
    src: &str,
    label_override: Option<&str>,
) -> String {
    let label = match label_override {
        Some(l) => l,
        None => {
            if diag.code.is_perf_note() {
                "note"
            } else if diag.code.is_warning() {
                "warning"
            } else {
                "error"
            }
        }
    };
    let lines: Vec<&str> = src.lines().collect();
    let idx = attr.line.saturating_sub(1);
    let mut context = Vec::new();
    if idx > 0
        && let Some(prev) = lines.get(idx - 1)
    {
        context.push((
            attr.line - 1,
            frontend::highlight::highlight_line(theme, prev),
        ));
    }
    if let Some(current) = lines.get(idx) {
        context.push((
            attr.line,
            frontend::highlight::highlight_line(theme, current),
        ));
    }
    if let Some(next) = lines.get(idx + 1) {
        context.push((
            attr.line + 1,
            frontend::highlight::highlight_line(theme, next),
        ));
    }
    let pointer_col = lines
        .get(idx)
        .map(|text| {
            let chars = text.chars().count();
            attr.col.saturating_sub(1).min(chars)
        })
        .unwrap_or(0);
    let footer = diag.hint.as_deref().map(|h| format!("note: {h}"));
    let footer2 = match crate::explain(diag.code.as_str()) {
        Some((_, fix)) if diag.hint.as_deref() != Some(fix.as_str()) => {
            Some(format!("fix: {}", render_fix_text(theme, &fix)))
        }
        _ => None,
    };
    let hook = diagnostics::tree_hook::TreeHook {
        title: format!("{label}[{}]: {}", diag.code.as_str(), diag.message),
        location: Some(format!("{}:{}:{}", attr.display, attr.line, attr.col)),
        context,
        pointer_line: attr.line,
        pointer_col,
        got: None,
        expected: None,
        footer,
        footer2,
    };
    diagnostics::tree_hook::render(theme, &hook)
}

pub fn render_compile_errors(
    theme: &diagnostics::theme::AuraTheme,
    entry: &Path,
    diags: &[Diagnostic],
) -> String {
    if let Ok(graph) = frontend::modules::ModuleGraph::build(entry) {
        let sources = load_sources(&graph);
        let mut out = String::new();
        for diag in diags {
            let attr = attribute(&graph, &sources, diag);
            let src = source_for(&sources, &attr.path);
            out.push_str(&render_diagnostic(theme, diag, &attr, &src));
        }
        return out;
    }
    render_lenient_errors(theme, entry, diags)
}

fn best_module(
    files: &[(PathBuf, String, frontend::ast::Module)],
    span: Option<Span>,
) -> usize {
    let modules: Vec<(&PathBuf, &frontend::ast::Module)> = files
        .iter()
        .map(|(path, _, module)| (path, module))
        .collect();
    if let Some(s) = span
        && let Some(i) = exact_span(&modules, s)
    {
        return i;
    }
    let offset = span.map(|s| s.start).unwrap_or(0);
    match tightest_span(&modules, offset) {
        Some((i, _)) => i,
        None => files
            .iter()
            .position(|(_, src, _)| (offset as usize) <= src.len())
            .unwrap_or(0),
    }
}

fn failed_owner(
    failed: &mut Vec<(PathBuf, String, Vec<(Code, Option<Span>)>)>,
    diag: &Diagnostic,
) -> Option<(PathBuf, String)> {
    for (path, src, remaining) in failed.iter_mut() {
        if let Some(i) = remaining
            .iter()
            .position(|(c, s)| *c == diag.code && *s == diag.span)
        {
            remaining.remove(i);
            return Some((path.clone(), src.clone()));
        }
    }
    None
}

pub fn discover_files(root: &Path) -> (Vec<(PathBuf, String, frontend::ast::Module)>, Vec<(PathBuf, String, Vec<(Code, Option<Span>)>)>) {
    let mut files = Vec::new();
    let mut failed = Vec::new();
    for path in crate::fmt::discover(root) {
        if !path.extension().is_some_and(|ext| ext == "rnx") {
            continue;
        }
        if let Ok(src) = std::fs::read_to_string(&path) {
            match frontend::parser::Parser::parse_module_all(&src) {
                Ok(module) => files.push((path, src, module)),
                Err(errs) => failed.push((
                    path,
                    src,
                    errs.iter().map(|e| (e.code, e.span)).collect(),
                )),
            }
        }
    }
    (files, failed)
}

pub fn project_root_for(entry: &Path) -> PathBuf {
    frontend::project::find_project_root(entry).unwrap_or_else(|| {
        entry
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    })
}

pub fn attribute_lenient(
    files: &[(PathBuf, String, frontend::ast::Module)],
    failed: &mut Vec<(PathBuf, String, Vec<(Code, Option<Span>)>)>,
    diag: &Diagnostic,
    entry: &Path,
) -> (Attributed, String) {
    if let Some(tagged) = diag.file.as_ref() {
        let src = std::fs::read_to_string(tagged).unwrap_or_default();
        if !src.is_empty() {
            let (line, col) = line_col(&src, diag.span.map(|s| s.start).unwrap_or(0));
            let attr = Attributed {
                path: tagged.clone(),
                display: relative_display(tagged),
                line,
                col,
            };
            return (attr, src);
        }
    }
    if let Some((path, src)) = failed_owner(failed, diag) {
        let (line, col) = line_col(&src, diag.span.map(|s| s.start).unwrap_or(0));
        let attr = Attributed {
            path: path.clone(),
            display: relative_display(&path),
            line,
            col,
        };
        return (attr, src);
    }
    let idx = best_module(files, diag.span).min(files.len().saturating_sub(1));
    if files.is_empty() {
        if let Some((path, src, _)) = failed.first() {
            let (line, col) = line_col(src, diag.span.map(|s| s.start).unwrap_or(0));
            let display = relative_display(path);
            return (
                Attributed {
                    path: path.clone(),
                    display,
                    line,
                    col,
                },
                src.clone(),
            );
        }
        let src = std::fs::read_to_string(entry).unwrap_or_default();
        let (line, col) = line_col(&src, diag.span.map(|s| s.start).unwrap_or(0));
        let display = relative_display(entry);
        return (
            Attributed {
                path: entry.to_path_buf(),
                display,
                line,
                col,
            },
            src,
        );
    }
    let (path, src, _) = &files[idx];
    let (line, col) = line_col(src, diag.span.map(|s| s.start).unwrap_or(0));
    let attr = Attributed {
        path: path.clone(),
        display: relative_display(path),
        line,
        col,
    };
    (attr, src.clone())
}

fn render_lenient_errors(
    theme: &diagnostics::theme::AuraTheme,
    entry: &Path,
    diags: &[Diagnostic],
) -> String {
    let root = project_root_for(entry);
    let (mut files, mut failed) = discover_files(&root);
    if files.is_empty() {
        let src = std::fs::read_to_string(entry).unwrap_or_default();
        files.push((
            entry.to_path_buf(),
            src,
            frontend::ast::Module {
                inner_attrs: Vec::new(),
                decls: Vec::new(),
                docs: String::new(),
                warnings: Vec::new(),
            },
        ));
    }
    let mut out = String::new();
    for diag in diags {
        let (attr, src) = attribute_lenient(&files, &mut failed, diag, entry);
        out.push_str(&render_diagnostic(theme, diag, &attr, &src));
    }
    out
}

fn func_file(
    files: &[(PathBuf, String, frontend::ast::Module)],
    root: &Path,
    func: &str,
) -> Option<usize> {
    let prefix = func.rfind('.').map(|i| &func[..i])?;
    let suffix = format!("/{}", prefix.replace("::", "/"));
    files.iter().position(|(path, _, _)| {
        let rel = path.strip_prefix(root).unwrap_or(path);
        let rel = rel.to_string_lossy().replace('\\', "/");
        let stem = rel.strip_suffix(".rnx").unwrap_or(&rel);
        stem == prefix.replace("::", "/") || stem.ends_with(&suffix)
    })
}

fn find_decl_span(module: &frontend::ast::Module, short: &str) -> Option<u32> {
    for decl in &module.decls {
        let name = match &decl.node {
            frontend::ast::Decl::Fn(f) => Some(f.name.as_str()),
            frontend::ast::Decl::Class { name, .. } => Some(name.as_str()),
            frontend::ast::Decl::Struct { name, .. } => Some(name.as_str()),
            frontend::ast::Decl::Trait { name, .. } => Some(name.as_str()),
            frontend::ast::Decl::Enum { name, .. } => Some(name.as_str()),
            frontend::ast::Decl::Record { name, .. } => Some(name.as_str()),
            frontend::ast::Decl::Const { name, .. } => Some(name.as_str()),
            _ => None,
        };
        if name.is_some_and(|n| n == short) {
            return Some(decl.span.start);
        }
    }
    None
}

fn render_box_at(
    theme: &diagnostics::theme::AuraTheme,
    title: &str,
    path: &Path,
    src: &str,
    line: usize,
    col: usize,
    pointer_col: usize,
) -> String {
    let lines: Vec<&str> = src.lines().collect();
    let line_idx = line.saturating_sub(1);
    let mut context = Vec::new();
    if line_idx > 0
        && let Some(prev) = lines.get(line_idx - 1)
    {
        context.push((
            line - 1,
            frontend::highlight::highlight_line(theme, prev),
        ));
    }
    if let Some(current) = lines.get(line_idx) {
        context.push((
            line,
            frontend::highlight::highlight_line(theme, current),
        ));
    }
    if let Some(next) = lines.get(line_idx + 1) {
        context.push((
            line + 1,
            frontend::highlight::highlight_line(theme, next),
        ));
    }
    let hook = diagnostics::tree_hook::TreeHook {
        title: title.to_string(),
        location: Some(format!("{}:{line}:{col}", relative_display(path))),
        context,
        pointer_line: line,
        pointer_col,
        got: None,
        expected: None,
        footer: None,
        footer2: None,
    };
    diagnostics::tree_hook::render(theme, &hook)
}

fn entry_file_fallback(
    files: &[(PathBuf, String, frontend::ast::Module)],
    entry: &Path,
) -> Option<(PathBuf, String)> {
    if let Some((path, src, _)) = files.iter().find(|(p, _, _)| p == entry) {
        return Some((path.clone(), src.clone()));
    }
    std::fs::read_to_string(entry)
        .ok()
        .map(|src| (entry.to_path_buf(), src))
}

pub fn render_runtime_error(
    theme: &diagnostics::theme::AuraTheme,
    title: &str,
    entry: &Path,
    span: Option<diagnostics::Span>,
    func: Option<&str>,
) -> String {
    let root = frontend::project::find_project_root(entry).unwrap_or_else(|| {
        entry
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| {
                std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
            })
    });
    let mut files: Vec<(PathBuf, String, frontend::ast::Module)> = Vec::new();
    for path in crate::fmt::discover(&root) {
        if !path.extension().is_some_and(|ext| ext == "rnx") {
            continue;
        }
        if let Ok(src) = std::fs::read_to_string(&path)
            && let Ok(module) = frontend::parser::Parser::parse_module(&src)
        {
            files.push((path, src, module));
        }
    }
    if let Some(func) = func
        && let Some(idx) = func_file(&files, &root, func)
    {
        let short = func.rsplit('.').next().unwrap_or(func);
        let short = short.split('.').next_back().unwrap_or(short);
        if let Some((path, src, module)) = files.get(idx) {
            let decl_start = find_decl_span(module, short);
            let (line, col, pointer_col) = match span {
                Some(span) if span != lir::instr::UNKNOWN_SPAN => {
                    let (line, col) = line_col(src, span.start);
                    let pointer_col = src
                        .lines()
                        .nth(line.saturating_sub(1))
                        .map(|text| text.chars().count().min(col.saturating_sub(1)))
                        .unwrap_or(0);
                    (line, col, pointer_col)
                }
                _ => match decl_start {
                    Some(offset) => {
                        let (line, _) = line_col(src, offset);
                        (line, 1, 0)
                    }
                    None => (1, 1, 0),
                },
            };
            return render_box_at(theme, title, path, src, line, col, pointer_col);
        }
    }
    if let Some(span) = span
        && span != lir::instr::UNKNOWN_SPAN
    {
        let entry_src = std::fs::read_to_string(entry).ok().or_else(|| {
            if entry.is_absolute() {
                None
            } else {
                std::env::current_dir()
                    .ok()
                    .and_then(|cwd| std::fs::read_to_string(cwd.join(entry)).ok())
            }
        });
        if let Some(src) = entry_src
            && (span.start as usize) <= src.len()
        {
            let (line, col) =
                diagnostics::line_col(&src, (span.start as usize).min(src.len()) as u32);
            let pointer_col = src
                .lines()
                .nth(line.saturating_sub(1))
                .map(|text| text.chars().count().min(col.saturating_sub(1)))
                .unwrap_or(0);
            return render_box_at(theme, title, entry, &src, line, col, pointer_col);
        }
    }
    if let Some(span) = span
        && span != lir::instr::UNKNOWN_SPAN
        && !files.is_empty()
    {
        let modules: Vec<(&PathBuf, &frontend::ast::Module)> = files
            .iter()
            .map(|(path, _, module)| (path, module))
            .collect();
        let offset = span.start;
        let idx = match exact_spans(&modules, span) {
            hits if hits.iter().any(|&i| files.get(i).is_some_and(|(p, _, _)| p == entry)) => {
                files.iter().position(|(p, _, _)| p == entry)
            }
            mut hits if hits.len() == 1 => hits.pop(),
            _ => tightest_span(&modules, offset).map(|(i, _)| i),
        };
        let resolved: Option<(PathBuf, String)> = match idx {
            Some(i) => files.get(i).map(|(p, s, _)| (p.clone(), s.clone())),
            None => entry_file_fallback(&files, entry),
        };
        if let Some((path, src)) = resolved {
            let (line, col) = line_col(&src, offset.min(src.len() as u32));
            let pointer_col = src
                .lines()
                .nth(line.saturating_sub(1))
                .map(|text| text.chars().count().min(col.saturating_sub(1)))
                .unwrap_or(0);
            return render_box_at(theme, title, &path, &src, line, col, pointer_col);
        }
    }
    let hook = diagnostics::tree_hook::TreeHook {
        title: title.to_string(),
        location: None,
        context: Vec::new(),
        pointer_line: 0,
        pointer_col: 0,
        got: None,
        expected: None,
        footer: None,
        footer2: None,
    };
    diagnostics::tree_hook::render(theme, &hook)
}

pub struct UncaughtFrame {
    pub func: String,
    pub file: String,
    pub line: usize,
    pub col: usize,
}

pub fn render_uncaught(message: &str, trace: &[UncaughtFrame]) -> String {
    let mut out = format!("Uncaught exception: {message}\n");
    if !trace.is_empty() {
        out.push_str("Stack trace:\n");
        for f in trace {
            out.push_str(&format!("  at {} ({}:{}:{})\n", f.func, f.file, f.line, f.col));
        }
    }
    out
}

pub fn uncaught_trace(
    entry: &Path,
    span: Option<diagnostics::Span>,
    func: Option<&str>,
) -> Vec<UncaughtFrame> {
    let (span, func) = match (span, func) {
        (Some(s), Some(f)) if s != lir::instr::UNKNOWN_SPAN => (s, f),
        _ => return Vec::new(),
    };
    let src = std::fs::read_to_string(entry).unwrap_or_default();
    if src.is_empty() {
        return Vec::new();
    }
    let (line, col) = diagnostics::line_col(&src, span.start.min(src.len() as u32));
    vec![UncaughtFrame {
        func: func.to_string(),
        file: relative_display(entry),
        line,
        col,
    }]
}

pub fn render_security_issues(diags: &[diagnostics::Diagnostic]) -> String {
    let mut out = String::new();
    for d in diags {
        out.push_str(&format!("error[{}]: {}", d.code.as_str(), d.message));
        if let Some(path) = d.file.as_ref() {
            let src = std::fs::read_to_string(path).unwrap_or_default();
            let (line, col) = diagnostics::line_col(
                &src,
                d.span.map(|s| s.start).unwrap_or(0),
            );
            out.push_str(&format!(" ({}:{}:{})", relative_display(path), line, col));
        }
        out.push('\n');
    }
    out
}

#[derive(Debug)]
pub struct DenyPerf {
    pub all: bool,
    pub codes: BTreeSet<String>,
}

impl DenyPerf {
    pub fn empty() -> DenyPerf {
        DenyPerf {
            all: false,
            codes: BTreeSet::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        !self.all && self.codes.is_empty()
    }

    pub fn denies(&self, code: &str) -> bool {
        self.all || self.codes.contains(code)
    }
}

pub fn parse_deny_perf(values: &[String]) -> Result<DenyPerf, String> {
    let mut deny = DenyPerf::empty();
    for value in values {
        for part in value.split(',') {
            let code = part.trim().to_ascii_uppercase();
            if code.is_empty() {
                continue;
            }
            if code == "ALL" {
                deny.all = true;
            } else if code == "P201" || code == "P202" || code == "P203" {
                deny.codes.insert(code);
            } else {
                return Err(format!(
                    "unknown --deny-perf-lints code `{part}`; expected P201, P202, P203 (or bare --deny-perf-lints for all)"
                ));
            }
        }
    }
    Ok(deny)
}

fn perf_tag(code: Code) -> Option<&'static str> {
    match code {
        Code::P101 => Some("PerfP101"),
        Code::P102 => Some("PerfP102"),
        Code::P201 => Some("PerfP201"),
        Code::P202 => Some("PerfP202"),
        Code::P203 => Some("PerfP203"),
        _ => None,
    }
}

fn allow_hits(attrs: &[frontend::ast::Attr], code: Code) -> bool {
    let tag = perf_tag(code);
    attrs.iter().any(|a| {
        a.name == "Allow"
            && a.args.iter().any(|arg| match &arg.node {
                frontend::ast::Expr::Ident(s) => s == "PerfAll" || Some(s.as_str()) == tag,
                _ => false,
            })
    })
}

fn unknown_perf_allows(module: &frontend::ast::Module) -> Vec<(Span, String)> {
    let mut out = Vec::new();
    let mut scan = |attrs: &[frontend::ast::Attr]| {
        for a in attrs {
            if a.name != "Allow" {
                continue;
            }
            for arg in &a.args {
                if let frontend::ast::Expr::Ident(s) = &arg.node {
                    if s == "PerfAll" || s == "PerfP101" || s == "PerfP102" || s == "PerfP201" || s == "PerfP202" || s == "PerfP203" {
                        continue;
                    }
                    if s.starts_with("Perf") {
                        out.push((a.span, s.clone()));
                    }
                }
            }
        }
    };
    scan(&module.inner_attrs);
    for decl in &module.decls {
        if let frontend::ast::Decl::Fn(f) = &decl.node {
            scan(&f.attrs);
        }
    }
    out
}

fn span_contains(outer: Span, inner: Span) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

fn perf_suppressed(
    files: &[(PathBuf, String, frontend::ast::Module)],
    idx: usize,
    diag: &Diagnostic,
) -> bool {
    let (_, _, module) = match files.get(idx) {
        Some(entry) => entry,
        None => return false,
    };
    if !diag.code.is_perf_lint() {
        return false;
    }
    if allow_hits(&module.inner_attrs, diag.code) {
        return true;
    }
    let span = match diag.span {
        Some(s) => s,
        None => return false,
    };
    for decl in &module.decls {
        if let frontend::ast::Decl::Fn(f) = &decl.node {
            if allow_hits(&f.attrs, diag.code) && span_contains(decl.span, span) {
                return true;
            }
        }
    }
    false
}

pub struct PerfRender {
    pub text: String,
    pub denied: Vec<Diagnostic>,
}

pub fn render_perf_stream(
    theme: &diagnostics::theme::AuraTheme,
    entry: &Path,
    diags: &[Diagnostic],
    deny: &DenyPerf,
    show_notes: bool,
) -> PerfRender {
    let entry_abs: PathBuf = if entry.is_absolute() {
        entry.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(entry)
    };
    let entry = entry_abs.as_path();
    let root = project_root_for(entry);
    let (files, mut failed) = discover_files(&root);
    let mut text = String::new();
    let mut denied = Vec::new();
    let mut unknowns: BTreeSet<(String, u32, u32, String)> = BTreeSet::new();
    for (path, _, module) in &files {
        for (span, arg) in unknown_perf_allows(module) {
            unknowns.insert((
                path.to_string_lossy().into_owned(),
                span.start,
                span.end,
                arg,
            ));
        }
    }
    for diag in diags {
        if !diag.code.is_perf_lint() {
            continue;
        }
        if diag.code.is_perf_note() && !show_notes {
            continue;
        }
        if files.is_empty() {
            continue;
        }
        let idx = best_module(&files, diag.span).min(files.len().saturating_sub(1));
        if perf_suppressed(&files, idx, diag) {
            continue;
        }
        let (attr, src) = attribute_lenient(&files, &mut failed, diag, entry);
        let is_denied = diag.code.is_warning() && deny.denies(diag.code.as_str());
        if is_denied {
            text.push_str(&render_diagnostic_with_label(
                theme,
                diag,
                &attr,
                &src,
                Some("error"),
            ));
            denied.push(diag.clone());
        } else {
            text.push_str(&render_diagnostic(theme, diag, &attr, &src));
        }
    }
    let mut unknowns: Vec<(String, u32, u32, String)> = unknowns.into_iter().collect();
    unknowns.sort();
    for (path, start, end, arg) in unknowns {
        let path = PathBuf::from(path);
        let src = std::fs::read_to_string(&path).unwrap_or_default();
        let span = Span { start, end };
        let (line, col) = line_col(&src, start);
        let warn = Diagnostic::new(
            Code::W201,
            format!(
                "unknown lint code `{arg}` in `Allow`; expected PerfP101, PerfP102, PerfP201, PerfP202, PerfP203, or PerfAll"
            ),
        )
        .with_span(span)
        .with_file(path.clone());
        let attr = Attributed {
            path: path.clone(),
            display: relative_display(&path),
            line,
            col,
        };
        text.push_str(&render_diagnostic(theme, &warn, &attr, &src));
    }
    denied.sort_by(|a, b| {
        a.code
            .as_str()
            .cmp(b.code.as_str())
            .then(a.span.cmp(&b.span))
    });
    PerfRender { text, denied }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> frontend::ast::Module {
        frontend::parser::Parser::parse_module(src).expect("fixture parses")
    }

    #[test]
    fn ambiguous_span_falls_back_to_entry() {
        let dir = std::env::temp_dir().join(format!("rnx-attr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = "fn Main(): Int {\n    return 0;\n}\n";
        let a_path = dir.join("a.rnx");
        let b_path = dir.join("b.rnx");
        std::fs::write(&a_path, src).unwrap();
        std::fs::write(&b_path, src).unwrap();
        let module = parse(src);
        let span = module.decls[0].span;
        let theme = diagnostics::theme::AuraTheme::plain();
        let out = render_runtime_error(&theme, "fatal: boom", &b_path, Some(span), None);
        assert!(out.contains("b.rnx"), "{out}");
        assert!(!out.contains("a.rnx"), "{out}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn best_module_prefers_exact_span_owner() {
        let main_src = "//! Mod docs here.\nimport {\n    Vec2\n} from \"./entities\";\n\nfn Main(): Int {\n    return 0;\n}\n";
        let ent_src = "//! Ents.\nimport {\n    Vec2\n} from \"@std/math \";\n\nclass Rock {\n    let id: Int;\n}\n";
        let files = vec![
            (PathBuf::from("src/main.rnx"), main_src.to_string(), parse(main_src)),
            (
                PathBuf::from("src/entities.rnx"),
                ent_src.to_string(),
                parse(ent_src),
            ),
        ];
        let ent_import = files[1]
            .2
            .decls
            .iter()
            .find(|d| matches!(&d.node, frontend::ast::Decl::Import(..)))
            .expect("entities has an import");
        let idx = best_module(&files, Some(ent_import.span));
        assert_eq!(idx, 1, "error must attribute to entities.rnx");
    }

    #[test]
    fn tightest_span_falls_back_sensibly() {
        let main_src = "fn Main(): Int {\n    return 0;\n}\n";
        let files = vec![(PathBuf::from("src/main.rnx"), main_src.to_string(), parse(main_src))];
        assert_eq!(best_module(&files, None), 0);
    }

    #[test]
    fn deny_perf_parses_shapes() {
        let d = parse_deny_perf(&[]).unwrap();
        assert!(d.is_empty());
        assert!(!d.denies("P201"));
        let d = parse_deny_perf(&["ALL".to_string()]).unwrap();
        assert!(!d.is_empty() && d.denies("P201") && d.denies("P202") && d.denies("P203"));
        let d = parse_deny_perf(&["P201".to_string(), "P203".to_string()]).unwrap();
        assert!(d.denies("P201") && d.denies("P203") && !d.denies("P202"));
        let d = parse_deny_perf(&["P201,P203".to_string()]).unwrap();
        assert!(d.denies("P201") && d.denies("P203") && !d.denies("P202"));
        let d = parse_deny_perf(&["p201".to_string()]).unwrap();
        assert!(d.denies("P201"));
        let err = parse_deny_perf(&["P999".to_string()]).unwrap_err();
        assert!(err.contains("P201, P202, P203"), "{err}");
        let err = parse_deny_perf(&["".to_string()]).unwrap();
        assert!(err.is_empty());
    }

    fn write_perf_case(tag: &str, src: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rnx-perf-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let main = dir.join("main.rnx");
        std::fs::write(&main, src).unwrap();
        main
    }

    fn perf_diag(code: Code, src: &str, needle: &str) -> Diagnostic {
        let start = src.find(needle).expect("needle") as u32;
        Diagnostic::new(code, format!("{code} probe")).with_span(Span {
            start,
            end: start + needle.len() as u32,
        })
    }

    const ALLOW_SRC: &str = "#[Allow(PerfP201)]\nfn Main(): Int {\n    let x = 1;\n    return x;\n}\nfn Other(): Int {\n    return 2;\n}\n";

    #[test]
    fn allow_on_item_suppresses_span_inside() {
        let main = write_perf_case("allow", ALLOW_SRC);
        let theme = diagnostics::theme::AuraTheme::plain();
        let in_main = perf_diag(Code::P201, ALLOW_SRC, "return x;");
        let in_other = perf_diag(Code::P201, ALLOW_SRC, "return 2;");
        let deny = DenyPerf::empty();
        let out_main = render_perf_stream(&theme, &main, &[in_main], &deny, false);
        assert!(out_main.text.is_empty(), "{}", out_main.text);
        let out_other = render_perf_stream(&theme, &main, &[in_other], &deny, false);
        assert!(
            out_other.text.contains("warning[P201]"),
            "{}",
            out_other.text
        );
        let _ = std::fs::remove_dir_all(main.parent().unwrap());
    }

    #[test]
    fn file_allow_suppresses_and_denies_only_unsuppressed() {
        let src = "#![Allow(PerfAll)]\nfn Main(): Int {\n    return 0;\n}\n";
        let main = write_perf_case("fileallow", src);
        let theme = diagnostics::theme::AuraTheme::plain();
        let diag = perf_diag(Code::P201, src, "return 0;");
        let deny = parse_deny_perf(&["ALL".to_string()]).unwrap();
        let out = render_perf_stream(&theme, &main, &[diag], &deny, false);
        assert!(out.text.is_empty(), "{}", out.text);
        assert!(out.denied.is_empty());
        let _ = std::fs::remove_dir_all(main.parent().unwrap());
    }

    #[test]
    fn denied_renders_as_error() {
        let src = "fn Main(): Int {\n    return 0;\n}\n";
        let main = write_perf_case("deny", src);
        let theme = diagnostics::theme::AuraTheme::plain();
        let diag = perf_diag(Code::P201, src, "return 0;");
        let deny = parse_deny_perf(&["P201".to_string()]).unwrap();
        let out = render_perf_stream(&theme, &main, &[diag], &deny, false);
        assert_eq!(out.denied.len(), 1);
        assert!(out.text.contains("error[P201]"), "{}", out.text);
        let note = perf_diag(Code::P101, src, "return 0;");
        let out = render_perf_stream(&theme, &main, &[note], &deny, true);
        assert!(out.denied.is_empty());
        assert!(out.text.contains("note[P101]"), "{}", out.text);
        let _ = std::fs::remove_dir_all(main.parent().unwrap());
    }

    #[test]
    fn notes_hidden_unless_shown() {
        let src = "fn Main(): Int {\n    return 0;\n}\n";
        let main = write_perf_case("notegate", src);
        let theme = diagnostics::theme::AuraTheme::plain();
        let note = perf_diag(Code::P101, src, "return 0;");
        let warn = perf_diag(Code::P201, src, "return 0;");
        let hidden = render_perf_stream(&theme, &main, &[note.clone(), warn.clone()], &DenyPerf::empty(), false);
        assert!(!hidden.text.contains("note[P101]"), "{}", hidden.text);
        assert!(hidden.text.contains("warning[P201]"), "{}", hidden.text);
        let shown = render_perf_stream(&theme, &main, &[note, warn], &DenyPerf::empty(), true);
        assert!(shown.text.contains("note[P101]"), "{}", shown.text);
        assert!(shown.text.contains("warning[P201]"), "{}", shown.text);
        let _ = std::fs::remove_dir_all(main.parent().unwrap());
    }

    #[test]
    fn unknown_perf_allow_warns() {
        let src = "#[Allow(PerfP999)]\nfn Main(): Int {\n    return 0;\n}\n";
        let main = write_perf_case("unknown", src);
        let theme = diagnostics::theme::AuraTheme::plain();
        let diag = perf_diag(Code::P201, src, "return 0;");
        let out = render_perf_stream(&theme, &main, &[diag], &DenyPerf::empty(), false);
        assert!(out.text.contains("warning[P201]"), "{}", out.text);
        assert!(out.text.contains("PerfP999"), "{}", out.text);
        let _ = std::fs::remove_dir_all(main.parent().unwrap());
    }
}
