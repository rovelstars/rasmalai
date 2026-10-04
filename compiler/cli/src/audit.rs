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
pub struct AuditReport {
    pub name: String,
    pub version: String,
    pub tier: String,
    pub capabilities: Vec<String>,
    pub has_hazard: bool,
    pub traces: Vec<AuditTrace>,
    pub summary: String,
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
