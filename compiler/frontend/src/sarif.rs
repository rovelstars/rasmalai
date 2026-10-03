use diagnostics::escape_json;

pub struct SarifResult {
    pub file: String,
    pub line: usize,
    pub col: usize,
    pub rule: &'static str,
    pub message: String,
}

fn rule_description(rule: &str) -> &'static str {
    match rule {
        "L001" => "unused variable",
        "L002" => "unused parameter",
        "L003" => "unreachable code",
        "L004" => "missing doc comment",
        "L005" => "empty block",
        _ => "lint finding",
    }
}

fn rules_json() -> String {
    let mut out = String::from("[");
    for (i, rule) in ["L001", "L002", "L003", "L004", "L005"].iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{{\"id\":\"{rule}\",\"name\":\"rasmalai-{rule}\",\"shortDescription\":{{\"text\":\"rasmalai/{rule}\"}},\"fullDescription\":{{\"text\":\"{}\"}},\"defaultConfiguration\":{{\"level\":\"warning\"}}}}",
            rule_description(rule)
        ));
    }
    out.push(']');
    out
}

pub fn sarif_report(results: &[SarifResult]) -> String {
    let mut out = String::from("{\"$schema\":\"https://json.schemastore.org/sarif-2.1.0.json\",\"version\":\"2.1.0\",\"runs\":[{\"tool\":{\"driver\":{\"name\":\"rnx lint\",\"informationUri\":\"https://rasmalai.dev\",\"rules\":");
    out.push_str(&rules_json());
    out.push_str("}},\"results\":[");
    for (i, result) in results.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{{\"ruleId\":\"{}\",\"level\":\"warning\",\"message\":{{\"text\":\"{}\"}},\"locations\":[{{\"physicalLocation\":{{\"artifactLocation\":{{\"uri\":\"{}\"}},\"region\":{{\"startLine\":{},\"startColumn\":{}}}}}}}]}}",
            result.rule,
            escape_json(&result.message),
            escape_json(&result.file),
            result.line,
            result.col,
        ));
    }
    out.push_str("]}]}");
    out
}
