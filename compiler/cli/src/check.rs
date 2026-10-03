use diagnostics::{JsonDiagnostic};
use std::path::{Path, PathBuf};

pub struct CheckOutcome {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}
fn package_name(entry: &Path, member: Option<&str>) -> String {
    if let Some(name) = member {
        return name.to_string();
    }
    let mut dir = entry.parent().map(|p| p.to_path_buf());
    while let Some(d) = dir {
        if let Ok(Some(cfg)) = frontend::project::ProjectConfig::load_from_dir(&d) {
            return cfg.name;
        }
        dir = d.parent().map(|p| p.to_path_buf());
    }
    entry
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| entry.to_string_lossy().into_owned())
}

fn is_manifest_path(path: &Path) -> bool {
    path.file_name().is_some_and(|n| {
        n == frontend::project::MANIFEST_FILE || n == frontend::deplock::LOCK_FILE
    })
}

fn lock_name_line(lock_text: &str, package: &str) -> usize {
    let needle = format!("\"{package}\"");
    for (i, line) in lock_text.lines().enumerate() {
        let t = line.trim();
        if t.starts_with("name:") && t.contains(&needle) {
            return i + 1;
        }
    }
    1
}

fn package_line_in_message(message: &str, lock_text: &str) -> usize {
    let mut rest = message;
    while let Some(start) = rest.find('`') {
        rest = &rest[start + 1..];
        if let Some(end) = rest.find('`') {
            let found = lock_name_line(lock_text, &rest[..end]);
            if found > 1 {
                return found;
            }
            rest = &rest[end + 1..];
        } else {
            break;
        }
    }
    1
}

struct ManifestReport<'a> {
    items: &'a mut Vec<JsonDiagnostic>,
    stderr: &'a mut String,
    json: bool,
    theme: diagnostics::theme::AuraTheme,
}

impl ManifestReport<'_> {
    fn push(
        &mut self,
        path: &Path,
        src: &str,
        line_0: usize,
        code: &str,
        error: bool,
        message: &str,
    ) -> bool {
        let line = line_0 + 1;
        if self.json {
            self.items.push(JsonDiagnostic {
                file: crate::report::relative_display(path),
                line,
                col: 1,
                code: code.to_string(),
                severity: if error {
                    "error".to_string()
                } else {
                    "warning".to_string()
                },
                message: message.to_string(),
            });
        } else {
            let label = if error { "error" } else { "warning" };
            let text = src
                .lines()
                .nth(line_0)
                .map(|l| {
                    frontend::highlight::highlight_line(&self.theme, l)
                })
                .unwrap_or_default();
            let hook = diagnostics::tree_hook::TreeHook {
                title: format!("{label}[{code}]: {message}"),
                location: Some(format!("{}:{line}:1", crate::report::relative_display(path))),
                context: vec![(line, text)],
                pointer_line: line,
                pointer_col: 0,
                got: None,
                expected: None,
                footer: None,
                footer2: None,
            };
            self.stderr
                .push_str(&diagnostics::tree_hook::render(&self.theme, &hook));
        }
        error
    }
}

fn check_manifest_root(report: &mut ManifestReport, root: &Path) -> bool {
    let mut failed = false;
    let manifest_path = root.join(frontend::project::MANIFEST_FILE);
    if manifest_path.is_file() {
        let manifest_text = std::fs::read_to_string(&manifest_path).unwrap_or_default();
        for issue in frontend::project::validate_manifest_text(&manifest_text, Some(root)) {
            failed |= report.push(
                &manifest_path,
                &manifest_text,
                issue.line,
                issue.code.as_str(),
                issue.error,
                &issue.message,
            );
        }
    }
    let lock_path = root.join(frontend::deplock::LOCK_FILE);
    if let Ok(lock_text) = std::fs::read_to_string(&lock_path) {
        match frontend::deplock::ProjectDepLock::parse(&lock_text) {
            Ok(lock) => {

                if let Ok(Some(config)) = frontend::project::ProjectConfig::load_from_dir(root)
                    && let Err(diag) = frontend::deplock::verify_lock(root, &config, &lock)
                {
                    failed = true;
                    let line = package_line_in_message(&diag.message, &lock_text);
                    report.push(
                        &lock_path,
                        &lock_text,
                        line.saturating_sub(1),
                        diag.code.as_str(),
                        true,
                        &diag.message,
                    );
                }
            }
            Err(msg) => {
                failed = true;
                report.push(&lock_path, &lock_text, 0, "E108", true, &msg);
            }
        }
    }
    failed
}

fn manifest_roots(targets: &[(PathBuf, Option<String>)]) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for (entry, _) in targets {
        let root = if is_manifest_path(entry) {
            entry.parent().map(|p| {
                std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
            })
        } else {
            frontend::project::find_project_root(entry)
        };
        if let Some(root) = root
            && !roots.contains(&root)
        {
            roots.push(root);
        }
    }
    roots
}

fn resolve_targets(paths: &[PathBuf], package: Option<&str>) -> Result<Vec<(PathBuf, Option<String>)>, String> {
    if paths.is_empty() {
        return match crate::resolve_scope_target(None, package) {
            Ok(target) => Ok(vec![(PathBuf::from(&target.entry), target.member)]),
            Err(e) => Err(format!("error: {e}")),
        };
    }
    let mut targets = Vec::new();
    for path in paths {
        if path.is_file() {
            targets.push((path.clone(), None));
        } else if path.is_dir() {
            match crate::resolve_scope_target(Some(&path.to_string_lossy()), None) {
                Ok(target) => targets.push((PathBuf::from(&target.entry), target.member)),
                Err(e) => return Err(format!("error: {e}")),
            }
        } else {
            return Err(format!("path not found: {}", path.display()));
        }
    }
    Ok(targets)
}

pub fn run_check(
    paths: &[PathBuf],
    package: Option<&str>,
    json: bool,
    quiet: bool,
) -> CheckOutcome {
    let mut stdout = String::new();
    let mut stderr = String::new();
    let fail = |stderr: String| CheckOutcome {
        code: 1,
        stdout: String::new(),
        stderr,
    };
    let targets = match resolve_targets(paths, package) {
        Ok(targets) => targets,
        Err(message) => return fail(format!("{message}\n")),
    };
    let mut items: Vec<JsonDiagnostic> = Vec::new();
    let mut failed = false;
    let mut pending_ok: Vec<String> = Vec::new();
    let mut verified: std::collections::HashSet<std::path::PathBuf> =
        std::collections::HashSet::new();
    let theme = diagnostics::theme::AuraTheme::active();
    for (entry, member) in &targets {
        if is_manifest_path(entry) {
            continue;
        }
        if entry.is_file()
            && !entry.extension().is_some_and(|ext| ext == "rnx")
        {
            stderr.push_str(&format!(
                "{}: not a checkable file (expected `.rnx`, `Project.config`, or `Project.deplock`)\n",
                crate::report::relative_display(entry)
            ));
            failed = true;
            continue;
        }
        let name = package_name(entry, member.as_deref());
        let (diags, is_err) = match frontend::check::check_package(entry) {
            Ok(warnings) => (warnings, false),
            Err(errors) => (errors, true),
        };
        let graph = frontend::modules::ModuleGraph::build(entry).ok();
        let (lenient_files, mut lenient_failed) = match &graph {
            Some(_) => (Vec::new(), Vec::new()),
            None => {
                let root = crate::report::project_root_for(entry);
                crate::report::discover_files(&root)
            }
        };
        let sources = match &graph {
            Some(g) => crate::report::load_sources(g),
            None => lenient_files
                .iter()
                .map(|(p, s, _)| (p.clone(), s.clone()))
                .collect(),
        };
        let mut lower_diags: Vec<diagnostics::Diagnostic> = Vec::new();
        if !is_err {
            if let Some(g) = &graph {
                match g.resolve() {
                    Ok(mut module) => {
                        frontend::harness::strip_tests(&mut module);
                        frontend::harness::strip_benches(&mut module);
                        let mut staged = frontend::desugar::desugar(&mut module);
                        match lir::lower::lower(&module) {
                            Ok(lowered) => staged.extend(lir::verify::verify(&lowered)),
                            Err(e) => staged.push(e),
                        }
                        lower_diags = staged;
                    }
                    Err(e) => lower_diags.push(e),
                }
            }
        }
        let mut target_failed = is_err;
        for diag in diags.iter().chain(lower_diags.iter()) {
            let (attr, lenient_src) = match &graph {
                Some(g) => {
                    let a = crate::report::attribute(g, &sources, diag);
                    (a, None)
                }
                None => {
                    let (a, s) = crate::report::attribute_lenient(
                        &lenient_files,
                        &mut lenient_failed,
                        diag,
                        entry,
                    );
                    (a, Some(s))
                }
            };
            if json {
                items.push(JsonDiagnostic {
                    file: attr.display,
                    line: attr.line,
                    col: attr.col,
                    code: diag.code.as_str().to_string(),
                    severity: if diag.code.is_warning() {
                        "warning".to_string()
                    } else {
                        "error".to_string()
                    },
                    message: diag.message.clone(),
                });
            } else {
                let src = match &lenient_src {
                    Some(s) => s.clone(),
                    None => crate::report::source_for(&sources, &attr.path),
                };
                stderr.push_str(&crate::report::render_diagnostic(&theme, diag, &attr, &src));
            }
            if !diag.code.is_warning() {
                target_failed = true;
            }
        }
        let canon = std::fs::canonicalize(entry).unwrap_or_else(|_| entry.clone());
        if verified.insert(canon) {
            let issues = frontend::security::verify_entry(entry);
            if !issues.is_empty() {
                target_failed = true;
                if json {
                    for d in &issues {
                        let (file, line, col) = match d.file.as_ref() {
                            Some(path) => {
                                let src = std::fs::read_to_string(path).unwrap_or_default();
                                let (line, col) = diagnostics::line_col(
                                    &src,
                                    d.span.map(|s| s.start).unwrap_or(0),
                                );
                                (
                                    crate::report::relative_display(path),
                                    line,
                                    col,
                                )
                            }
                            None => (String::new(), 0, 0),
                        };
                        items.push(diagnostics::JsonDiagnostic {
                            file,
                            line,
                            col,
                            code: d.code.as_str().to_string(),
                            severity: "error".to_string(),
                            message: d.message.clone(),
                        });
                    }
                } else {
                    stderr.push_str(&crate::report::render_security_issues(&issues));
                }
            }
        }
        if !target_failed {
            pending_ok.push(name.clone());
        } else {
            failed = true;
        }
    }
    for root in manifest_roots(&targets) {
        let mut report = ManifestReport {
            items: &mut items,
            stderr: &mut stderr,
            json,
            theme,
        };
        failed |= check_manifest_root(&mut report, &root);
    }
    if !json && !quiet && !failed {
        for name in &pending_ok {
            stderr.push_str(&format!("Checked {name} (ok)\n"));
        }
    }
    if json {
        stdout.push_str(&diagnostics::diagnostics_to_json(&items));
        stdout.push('\n');
    }
    CheckOutcome {
        code: i32::from(failed),
        stdout,
        stderr,
    }
}
