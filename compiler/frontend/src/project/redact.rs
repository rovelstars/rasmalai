use diagnostics::{Code, Diagnostic};

pub const REDACTED_VERSION: &str = "0.0.0-published";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishFacts {
    pub name: String,
    pub version: String,
    pub redacted: String,
    pub registry_deps: Vec<(String, String)>,
}

fn is_key_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

struct Scan<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Scan<'a> {
    fn skip_ws_comments(&mut self) {
        loop {
            while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
                self.pos += 1;
            }
            if self.pos + 1 < self.bytes.len() && self.bytes[self.pos] == b'/' {
                if self.bytes[self.pos + 1] == b'/' {
                    while self.pos < self.bytes.len() && self.bytes[self.pos] != b'\n' {
                        self.pos += 1;
                    }
                    continue;
                }
                if self.bytes[self.pos + 1] == b'*' {
                    self.pos += 2;
                    while self.pos + 1 < self.bytes.len()
                        && !(self.bytes[self.pos] == b'*' && self.bytes[self.pos + 1] == b'/')
                    {
                        self.pos += 1;
                    }
                    self.pos = (self.pos + 2).min(self.bytes.len());
                    continue;
                }
            }
            break;
        }
    }

    fn peek_key(&mut self) -> Option<(String, usize, usize)> {
        self.skip_ws_comments();
        let start = self.pos;
        if self.pos < self.bytes.len() && self.bytes[self.pos] == b'"' {
            self.pos += 1;
            let mut key = String::new();
            while self.pos < self.bytes.len() {
                let b = self.bytes[self.pos];
                if b == b'\\' {
                    if self.pos + 1 < self.bytes.len() {
                        key.push(self.bytes[self.pos + 1] as char);
                        self.pos += 2;
                        continue;
                    }
                    return None;
                }
                if b == b'"' {
                    self.pos += 1;
                    return Some((key, start, self.pos));
                }
                if b < 0x20 {
                    return None;
                }
                key.push(b as char);
                self.pos += 1;
            }
            return None;
        }
        let mut end = self.pos;
        while end < self.bytes.len() && is_key_byte(self.bytes[end]) {
            end += 1;
        }
        if end == start {
            return None;
        }
        let key = std::str::from_utf8(&self.bytes[start..end]).ok()?.to_string();
        self.pos = end;
        Some((key, start, end))
    }

    fn eat(&mut self, b: u8) -> bool {
        self.skip_ws_comments();
        if self.pos < self.bytes.len() && self.bytes[self.pos] == b {
            self.pos += 1;
            return true;
        }
        false
    }

    fn string_span(&mut self) -> Option<(usize, usize)> {
        self.skip_ws_comments();
        if self.pos >= self.bytes.len() || self.bytes[self.pos] != b'"' {
            return None;
        }
        let start = self.pos;
        self.pos += 1;
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            if b == b'\\' {
                self.pos += 2;
                continue;
            }
            if b == b'"' {
                self.pos += 1;
                return Some((start, self.pos));
            }
            self.pos += 1;
        }
        None
    }

    fn skip_value(&mut self) -> bool {
        self.skip_ws_comments();
        if self.pos >= self.bytes.len() {
            return false;
        }
        match self.bytes[self.pos] {
            b'"' => self.string_span().is_some(),
            b'{' => {
                self.pos += 1;
                let mut depth = 1usize;
                while self.pos < self.bytes.len() && depth > 0 {
                    match self.bytes[self.pos] {
                        b'"' => {
                            if self.string_span().is_none() {
                                return false;
                            }
                            continue;
                        }
                        b'/' if self.pos + 1 < self.bytes.len()
                            && (self.bytes[self.pos + 1] == b'/'
                                || self.bytes[self.pos + 1] == b'*') =>
                        {
                            self.skip_ws_comments();
                            continue;
                        }
                        b'{' => depth += 1,
                        b'}' => depth -= 1,
                        _ => {}
                    }
                    self.pos += 1;
                }
                depth == 0
            }
            b'[' => {
                self.pos += 1;
                let mut depth = 1usize;
                while self.pos < self.bytes.len() && depth > 0 {
                    match self.bytes[self.pos] {
                        b'"' => {
                            if self.string_span().is_none() {
                                return false;
                            }
                            continue;
                        }
                        b'/' if self.pos + 1 < self.bytes.len()
                            && (self.bytes[self.pos + 1] == b'/'
                                || self.bytes[self.pos + 1] == b'*') =>
                        {
                            self.skip_ws_comments();
                            continue;
                        }
                        b'[' => depth += 1,
                        b']' => depth -= 1,
                        _ => {}
                    }
                    self.pos += 1;
                }
                depth == 0
            }
            _ => {
                while self.pos < self.bytes.len()
                    && !matches!(self.bytes[self.pos], b',' | b'}' | b']')
                {
                    if self.bytes[self.pos] == b'"' {
                        if self.string_span().is_none() {
                            return false;
                        }
                        continue;
                    }
                    self.pos += 1;
                }
                true
            }
        }
    }
}

fn version_spans_in_project_table(text: &str) -> Result<Vec<(usize, usize)>, Diagnostic> {
    let bytes = text.as_bytes();
    let mut scan = Scan { bytes, pos: 0 };
    scan.skip_ws_comments();
    if let Some((k, _, _)) = scan.peek_key() {
        if k == "export" {
            if scan.peek_key().map(|(k, _, _)| k).unwrap_or_default() == "default" {
                if !scan.eat(b'{') {
                    return Err(Diagnostic::new(
                        Code::E108,
                        "cannot redact Project.config: expected `{` after `export default`",
                    ));
                }
            } else {
                return Err(Diagnostic::new(
                    Code::E108,
                    "cannot redact Project.config: expected `default` after `export`",
                ));
            }
        } else {
            return Err(Diagnostic::new(
                Code::E108,
                "cannot redact Project.config: expected `export default`",
            ));
        }
    } else {
        return Err(Diagnostic::new(
            Code::E108,
            "cannot redact Project.config: expected `export default`",
        ));
    }
    let mut project_span: Option<(usize, usize)> = None;
    loop {
        scan.skip_ws_comments();
        if scan.pos < bytes.len() && bytes[scan.pos] == b'}' {
            break;
        }
        let (key, _, _) = scan.peek_key().ok_or_else(|| {
            Diagnostic::new(Code::E108, "cannot redact Project.config: malformed top-level table")
        })?;
        if !scan.eat(b':') && !scan.eat(b'=') {
            return Err(Diagnostic::new(
                Code::E108,
                "cannot redact Project.config: expected `:` after top-level key",
            ));
        }
        if key == "project" {
            scan.skip_ws_comments();
            if scan.pos >= bytes.len() || bytes[scan.pos] != b'{' {
                return Err(Diagnostic::new(
                    Code::E108,
                    "cannot redact Project.config: `project` must be a table",
                ));
            }
            let start = scan.pos;
            if !scan.skip_value() {
                return Err(Diagnostic::new(
                    Code::E108,
                    "cannot redact Project.config: malformed `project` table",
                ));
            }
            project_span = Some((start, scan.pos));
        } else if !scan.skip_value() {
            return Err(Diagnostic::new(
                Code::E108,
                "cannot redact Project.config: malformed top-level value",
            ));
        }
        scan.skip_ws_comments();
        if scan.pos < bytes.len() && bytes[scan.pos] == b',' {
            scan.pos += 1;
        }
    }
    let (start, end) = project_span.ok_or_else(|| {
        Diagnostic::new(Code::E108, "cannot redact Project.config: no `project` table")
    })?;
    let mut inner = Scan { bytes, pos: start + 1 };
    let mut spans = Vec::new();
    loop {
        inner.skip_ws_comments();
        if inner.pos >= end - 1 {
            break;
        }
        let (key, _, _) = inner.peek_key().ok_or_else(|| {
            Diagnostic::new(Code::E108, "cannot redact Project.config: malformed `project` table")
        })?;
        if !inner.eat(b':') && !inner.eat(b'=') {
            return Err(Diagnostic::new(
                Code::E108,
                "cannot redact Project.config: expected `:` after `project` key",
            ));
        }
        if key == "version" {
            let (s, e) = inner.string_span().ok_or_else(|| {
                Diagnostic::new(Code::E108, "cannot redact Project.config: `version` must be a string")
            })?;
            spans.push((s, e));
        } else if !inner.skip_value() {
            return Err(Diagnostic::new(
                Code::E108,
                "cannot redact Project.config: malformed `project` value",
            ));
        }
        inner.skip_ws_comments();
        if inner.pos < bytes.len() && bytes[inner.pos] == b',' {
            inner.pos += 1;
        }
    }
    if spans.is_empty() {
        return Err(Diagnostic::new(
            Code::E108,
            "cannot redact Project.config: no `version` in `project` table",
        ));
    }
    Ok(spans)
}

fn replace_spans(text: &str, mut spans: Vec<(usize, usize)>, replacement: &str) -> String {
    spans.sort();
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;
    for (s, e) in spans {
        out.push_str(&text[cursor..s]);
        out.push('"');
        out.push_str(replacement);
        out.push('"');
        cursor = e;
    }
    out.push_str(&text[cursor..]);
    out
}

pub fn redact_project_version(text: &str) -> Result<String, Diagnostic> {
    let spans = version_spans_in_project_table(text)?;
    Ok(replace_spans(text, spans, REDACTED_VERSION))
}

pub fn rehydrate_project_version(text: &str, version: &str) -> Result<String, Diagnostic> {
    let spans = version_spans_in_project_table(text)?;
    for (s, e) in &spans {
        if &text[*s..*e] != format!("\"{REDACTED_VERSION}\"") {
            return Err(Diagnostic::new(
                Code::E108,
                "cannot rehydrate Project.config: version is not the published placeholder",
            ));
        }
    }
    Ok(replace_spans(text, spans, version))
}

pub fn publish_facts(text: &str) -> Result<PublishFacts, Diagnostic> {
    let (project, _) =
        super::parse_manifest(text).map_err(|e| Diagnostic::new(Code::E108, e))?;
    let cfg = project.ok_or_else(|| {
        Diagnostic::new(Code::E108, "Project.config has no `project` table")
    })?;
    let version = cfg.version.clone();
    let name = cfg.name.clone();
    let mut registry_deps = Vec::new();
    for (dep, spec) in &cfg.dependencies {
        match spec {
            super::DependencySpec::Semver { version } => {
                registry_deps.push((dep.clone(), version.clone()));
            }
            super::DependencySpec::Url { url, .. } => {
                registry_deps.push((dep.clone(), url.clone()));
            }
            super::DependencySpec::Path { .. }
            | super::DependencySpec::Git { .. }
            | super::DependencySpec::Native { .. } => {}
        }
    }
    let redacted = redact_project_version(text)?;
    let (re_project, _) =
        super::parse_manifest(&redacted).map_err(|e| Diagnostic::new(Code::E108, e))?;
    let re_cfg = re_project.ok_or_else(|| {
        Diagnostic::new(Code::E108, "redacted Project.config has no `project` table")
    })?;
    if re_cfg.version != REDACTED_VERSION {
        return Err(Diagnostic::new(
            Code::E108,
            "Project.config redaction missed the published version",
        ));
    }
    if re_cfg.name != name {
        return Err(Diagnostic::new(
            Code::E108,
            "Project.config redaction changed the package name",
        ));
    }
    if re_cfg.entries.main != cfg.entries.main
        || re_cfg.entries.lib != cfg.entries.lib
        || re_cfg.entries.docs != cfg.entries.docs
        || re_cfg.entries.bins != cfg.entries.bins
    {
        return Err(Diagnostic::new(
            Code::E108,
            "Project.config redaction changed the entries manifest",
        ));
    }
    Ok(PublishFacts { name, version, redacted, registry_deps })
}
