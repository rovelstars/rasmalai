use diagnostics::Diagnostic;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct AuditTraceNode {
    pub file: String,
    pub line: usize,
    pub col: usize,
    pub symbol: String,
    pub expression_snippet: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuditTrace {
    pub capability: String,
    pub is_delegated: bool,
    pub nodes: Vec<AuditTraceNode>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BoundaryRefusal {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ForeignBoundary {
    pub libs: Vec<String>,
    pub used: Vec<String>,
    pub refused: Vec<BoundaryRefusal>,
    pub isolation: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuditReport {
    pub name: String,
    pub version: String,
    pub tier: String,
    pub capabilities: Vec<String>,
    pub has_hazard: bool,
    pub traces: Vec<AuditTrace>,
    pub summary: String,
    pub boundary: ForeignBoundary,
}

#[derive(Debug, Clone, Serialize)]
pub struct CapabilityManifest {
    pub name: String,
    pub version: String,
    pub tier: String,
    pub capabilities: Vec<String>,
}

fn is_hazard_cap(cap: &str) -> bool {
    cap.starts_with("sys:exec:") || cap.starts_with("unsafe:")
}

fn imported_fn_names(clause: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (i, _) in clause.match_indices("fn ") {
        let before = clause[..i].chars().next_back();
        if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let name: String = clause[i + 3..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() {
            out.push(name);
        }
    }
    out
}

fn native_imports(text: &str) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    let mut rest = text;
    loop {
        let start = match rest.find("from native") {
            Some(i) => i,
            None => break,
        };
        let head = &rest[..start];
        rest = &rest[start + "from native".len()..];
        let quote = match rest.find('"') {
            Some(i) => i,
            None => break,
        };
        let tail = &rest[quote + 1..];
        let end = match tail.find('"') {
            Some(i) => i,
            None => break,
        };
        let lib = tail[..end].to_string();
        rest = &tail[end + 1..];
        let mut fns = Vec::new();
        if let Some(open) = head.rfind("import {").or_else(|| head.rfind("export {")) {
            fns = imported_fn_names(&head[open..]);
        }
        out.push((lib, fns));
    }
    out
}

fn advisory_refusal(reason: &str) -> bool {
    matches!(reason, "variadic" | "callback" | "voidptr" | "va_list")
}

fn advisory_refusals(used: &[&str], skipped: &[(String, String)]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for name in used {
        for (skipped_name, reason) in skipped {
            if skipped_name == name && advisory_refusal(reason) {
                out.push(((*name).to_string(), reason.clone()));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn boundary_profile_skipped(dir: &Path) -> Vec<(String, String)> {
    let raw = match std::fs::read_to_string(dir.join("bindgen.oci.json")) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let doc: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    if let Some(skipped) = doc.get("profile").and_then(|p| p.get("skipped")).and_then(|s| s.as_array()) {
        for item in skipped {
            let name = item.get("name").and_then(|n| n.as_str()).unwrap_or_default();
            let reason = item.get("reason").and_then(|r| r.as_str()).unwrap_or_default();
            if !name.is_empty() && !reason.is_empty() {
                out.push((name.to_string(), reason.to_string()));
            }
        }
    }
    out
}

pub fn boundary_for_dir(dir: &Path) -> ForeignBoundary {
    let mut libs: Vec<String> = Vec::new();
    let mut used: Vec<String> = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(top) = stack.pop() {
        let mut entries = match std::fs::read_dir(&top) {
            Ok(e) => e.flatten().map(|e| e.path()).collect::<Vec<_>>(),
            Err(_) => continue,
        };
        entries.sort();
        for path in entries {
            if path.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                if name == "target" || name.starts_with('.') {
                    continue;
                }
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rnx") {
                continue;
            }
            let text = match std::fs::read_to_string(&path) {
                Ok(t) => t,
                Err(_) => continue,
            };
            for (lib, fns) in native_imports(&text) {
                libs.push(lib);
                used.extend(fns);
            }
        }
    }
    libs.sort();
    libs.dedup();
    used.sort();
    used.dedup();
    let skipped = boundary_profile_skipped(dir);
    let used_refs: Vec<&str> = used.iter().map(|s| s.as_str()).collect();
    let refused = advisory_refusals(&used_refs, &skipped)
        .into_iter()
        .map(|(name, reason)| BoundaryRefusal { name, reason })
        .collect::<Vec<_>>();
    let isolation = if refused.is_empty() { "proxyable".to_string() } else { "none".to_string() };
    ForeignBoundary {
        libs,
        used,
        refused,
        isolation,
        note: "advisory transparency report: the audit itself is the map; nothing here is enforced".to_string(),
    }
}

pub fn audit_path(dir: &Path) -> Result<AuditReport, Diagnostic> {
    let analysis = frontend::security::analyze_dep_dir(dir)?;
    let tier = analysis.tier.to_string();
    let has_hazard = analysis.capabilities.iter().any(|c| is_hazard_cap(c));
    let traces: Vec<AuditTrace> = analysis
        .traces
        .iter()
        .map(|t| AuditTrace {
            capability: t.capability.to_string(),
            is_delegated: t.is_delegated,
            nodes: t
                .nodes
                .iter()
                .map(|n| AuditTraceNode {
                    file: n.file.clone(),
                    line: n.line,
                    col: n.col,
                    symbol: n.symbol.clone(),
                    expression_snippet: n.expression_snippet.clone(),
                })
                .collect(),
        })
        .collect();
    let summary = if analysis.capabilities.is_empty() {
        format!(
            "{} v{}: pure computation, no capabilities required",
            analysis.name, analysis.version,
        )
    } else {
        format!(
            "{} v{}: tier {tier}, {} {}",
            analysis.name,
            analysis.version,
            analysis.capabilities.len(),
            if analysis.capabilities.len() == 1 {
                "capability"
            } else {
                "capabilities"
            },
        )
    };
    Ok(AuditReport {
        name: analysis.name,
        version: analysis.version,
        tier,
        capabilities: analysis.capabilities,
        has_hazard,
        traces,
        summary,
        boundary: boundary_for_dir(dir),
    })
}

pub fn tier_badge(tier: &str) -> &'static str {
    match tier {
        "pure" => "[TIER 1: PURE]",
        "delegated" => "[TIER 2: DELEGATED]",
        "ambient" => "[TIER 3: AMBIENT]",
        "hazard" => "[TIER 4: HAZARD]",
        _ => "[TIER ?: UNKNOWN]",
    }
}

pub fn tier_blurb(tier: &str) -> &'static str {
    match tier {
        "pure" => "Zero capabilities required",
        "delegated" => "I/O strictly on arguments passed by caller",
        "ambient" => "Static endpoints, ambient files, or environment variables",
        "hazard" => "Spawns child processes or uses raw memory/FFI",
        _ => "Unrecognized tier",
    }
}

pub fn render_audit_text(report: &AuditReport, drift: &[String]) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{} v{} {} ({})\n",
        report.name,
        report.version,
        tier_badge(&report.tier),
        tier_blurb(&report.tier),
    ));
    if report.capabilities.is_empty() {
        out.push_str("No external capabilities detected. 100% verified pure computation.\n");
    } else {
        for trace in &report.traces {
            out.push_str(&format!("[!] {}\n", trace.capability));
            let mut nodes = trace.nodes.iter().peekable();
            if let Some(origin) = nodes.next() {
                out.push_str(&format!(
                    "    {}:{} ({})\n",
                    origin.file, origin.line, origin.symbol,
                ));
            }
            for node in nodes {
                out.push_str(&format!(
                    "      -> {} ({}:{})\n",
                    node.symbol, node.file, node.line,
                ));
            }
            if trace.is_delegated {
                out.push_str("    (delegated: operates only on caller-provided values)\n");
            }
        }
    }
    if !drift.is_empty() {
        out.push_str("lockfile drift:\n");
        for line in drift {
            out.push_str(&format!("  [!] {line}\n"));
        }
    }
    out.push_str("foreign boundary (advisory transparency report):\n");
    if report.boundary.libs.is_empty() {
        out.push_str("  no `from native` imports\n");
    } else {
        out.push_str(&format!("  libs: {}\n", report.boundary.libs.join(", ")));
        out.push_str(&format!("  isolation: {}\n", report.boundary.isolation));
        if report.boundary.refused.is_empty() {
            out.push_str("  refused: none\n");
        } else {
            for r in &report.boundary.refused {
                out.push_str(&format!("  [!] refused: `{}` ({})\n", r.name, r.reason));
            }
        }
    }
    out
}

pub fn export_manifest(report: &AuditReport) -> String {
    let manifest = CapabilityManifest {
        name: report.name.clone(),
        version: report.version.clone(),
        tier: report.tier.clone(),
        capabilities: report.capabilities.clone(),
    };
    serde_json::to_string_pretty(&manifest).unwrap_or_else(|_| "{}".to_string())
}

pub fn report_json(report: &AuditReport) -> String {
    serde_json::to_string_pretty(report).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rnx-audit-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn native_import_names_parse() {
        let found = native_imports(
            "import { fn strlen(s: Pointer<Byte>): Int, fn free(p: Pointer<Byte>): Void as release } from native \"c\"\n",
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "c");
        assert_eq!(found[0].1, vec!["strlen".to_string(), "free".to_string()]);
        assert!(native_imports("fn Main(): Int { return 0; }\n").is_empty());
    }

    #[test]
    fn boundary_reports_refused_surface_as_advisory() {
        let dir = scratch("boundary");
        std::fs::write(
            dir.join("main.rnx"),
            "import { fn cb(f: Pointer<Byte>): Int } from native \"evil\"\nfn Main(): Int { return 0; }\n",
        )
        .unwrap();
        let boundary = boundary_for_dir(&dir);
        assert_eq!(boundary.libs, vec!["evil".to_string()]);
        assert_eq!(boundary.refused.len(), 0);
        assert_eq!(boundary.isolation, "proxyable");
        assert!(boundary.note.contains("audit itself is the map"), "{}", boundary.note);
        std::fs::write(
            dir.join("bindgen.oci.json"),
            "{\"profile\": {\"skipped\": [{\"name\": \"cb\", \"reason\": \"callback\"}]}}",
        )
        .unwrap();
        let boundary = boundary_for_dir(&dir);
        assert_eq!(boundary.refused.len(), 1);
        assert_eq!(boundary.refused[0].name, "cb");
        assert_eq!(boundary.isolation, "none");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

pub fn lockfile_drift(dir: &Path) -> Vec<String> {
    let config = match frontend::project::ProjectConfig::load_from_dir(dir) {
        Ok(Some(c)) => c,
        _ => return Vec::new(),
    };
    if frontend::deplock::ProjectDepLock::load(dir)
        .ok()
        .flatten()
        .is_none()
    {
        return vec!["no Project.deplock found; run `rnx lock`".to_string()];
    }
    let entry = config.main_path(dir);
    let target = if entry.is_file() { entry } else { dir.to_path_buf() };
    frontend::security::verify_entry(&target)
        .into_iter()
        .map(|d| format!("[{}] {}", d.code.as_str(), d.message))
        .collect()
}

pub enum AdvisoryCheck {
    Skipped(String),
    Rejected(String),
    Done(Vec<frontend::advisory::Finding>),
}

pub fn advisory_registry(flag: Option<&str>) -> String {
    let from_flag = flag.map(|s| s.to_string());
    let from_env = std::env::var("RNX_REGISTRY").ok().filter(|s| !s.trim().is_empty());
    match from_flag.or(from_env) {
        Some(url) => frontend::fetch::expand_registry_base(&url),
        None => frontend::fetch::expand_registry_base(""),
    }
}

pub fn audit_advisories(dir: &Path, registry: &str) -> AdvisoryCheck {
    let lock = match frontend::deplock::ProjectDepLock::load(dir) {
        Ok(Some(lock)) => lock,
        Ok(None) => {
            return AdvisoryCheck::Skipped("no Project.deplock found; skipping advisory check".to_string());
        }
        Err(e) => {
            return AdvisoryCheck::Skipped(format!("cannot read Project.deplock: {}; skipping advisory check", e.message));
        }
    };
    match frontend::advisory::fetch_signed_feed(registry) {
        Ok(feed) => AdvisoryCheck::Done(frontend::advisory::match_lock(&lock, &feed.advisories)),
        Err(frontend::advisory::FeedError::Unavailable(message)) => {
            AdvisoryCheck::Skipped(format!("advisory feed unavailable ({message}); skipping advisory check"))
        }
        Err(frontend::advisory::FeedError::Rejected(message)) => {
            AdvisoryCheck::Rejected(format!("advisory feed rejected: {message}"))
        }
    }
}

pub fn render_advisory_text(findings: &[frontend::advisory::Finding]) -> String {
    if findings.is_empty() {
        return "advisories: no advisories apply to the locked dependencies.\n".to_string();
    }
    let mut out = String::from("advisories:\n");
    for finding in findings {
        let pin = if finding.pinned_artifact { " (pinned artifact)" } else { "" };
        match finding.severity.as_str() {
            "critical" => {
                out.push_str(&format!(
                    "  error: advisory {} [critical security hold]: {}@{}{}: {}\n",
                    finding.advisory_id, finding.package, finding.version, pin, finding.title,
                ));
            }
            "warning" => {
                out.push_str(&format!(
                    "  warning: package `{}@{}` is affected by advisory {}{}: {}\n",
                    finding.package, finding.version, finding.advisory_id, pin, finding.title,
                ));
            }
            _ => {
                out.push_str(&format!(
                    "  note: advisory {} [info]: {}@{}{}: {}\n",
                    finding.advisory_id, finding.package, finding.version, pin, finding.title,
                ));
            }
        }
        if !finding.detail.is_empty() {
            out.push_str(&format!("    {}\n", finding.detail.replace('\n', "\n    ")));
        }
    }
    let holds = findings.iter().filter(|f| f.severity == "critical").count();
    let warnings = findings.iter().filter(|f| f.severity == "warning").count();
    out.push_str(&format!(
        "advisory check: {} {} ({} critical, {} warning)\n",
        findings.len(),
        if findings.len() == 1 { "finding" } else { "findings" },
        holds,
        warnings,
    ));
    out
}
