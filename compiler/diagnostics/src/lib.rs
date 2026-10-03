use std::fmt;

pub mod theme;
pub mod tree_hook;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: u32, end: u32) -> Option<Span> {
        if start <= end {
            Some(Span { start, end })
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Code {
    E005,
    E105,
    E107,
    E108,
    E109,
    E110,
    E111,
    E112,
    E201,
    E202,
    E203,
    E204,
    E205,
    E206,
    E302,
    E303,
    E304,
    E305,
    E402,
    E501,
    W104,
    W108,
    W109,
    W201,
    W204,
    S101,
    S102,
    S201,
    S301,
    S401,
    S501,
}

impl Code {
    pub fn as_str(self) -> &'static str {
        match self {
            Code::E005 => "E005",
            Code::E105 => "E105",
            Code::E107 => "E107",
            Code::E108 => "E108",
            Code::E109 => "E109",
            Code::E110 => "E110",
            Code::E111 => "E111",
            Code::E112 => "E112",
            Code::E201 => "E201",
            Code::E202 => "E202",
            Code::E203 => "E203",
            Code::E204 => "E204",
            Code::E205 => "E205",
            Code::E206 => "E206",
            Code::E302 => "E302",
            Code::E303 => "E303",
            Code::E304 => "E304",
            Code::E305 => "E305",
            Code::E402 => "E402",
            Code::E501 => "E501",
            Code::S101 => "S101",
            Code::S102 => "S102",
            Code::S201 => "S201",
            Code::S301 => "S301",
            Code::S401 => "S401",
            Code::S501 => "S501",
            Code::W104 => "W104",
            Code::W108 => "W108",
            Code::W109 => "W109",
            Code::W201 => "W201",
            Code::W204 => "W204",
        }
    }

    pub fn is_warning(self) -> bool {
        matches!(self, Code::W104 | Code::W108 | Code::W109 | Code::W201 | Code::W204)
    }

    pub fn title(self) -> &'static str {
        match self {
            Code::E005 => "removed double-colon syntax",
            Code::E105 => "invalid lambda syntax",
            Code::E107 => "circular module dependency",
            Code::E108 => "general compile error",
            Code::E109 => "await outside async function",
            Code::E110 => "C-style for loop is not supported",
            Code::E111 => "direct poll() invocation",
            Code::E112 => "top-level imperative statement in imported module",
            Code::E201 => "call to unsafe function outside unsafe block",
            Code::E202 => "raw pointer use outside unsafe block",
            Code::E203 => "private member accessed outside its class",
            Code::E204 => "class invoked without `new`",
            Code::E205 => "assignment type mismatch",
            Code::E206 => "variable declaration must be initialized with an expression",
            Code::E302 => "intra-procedural ARC cycle",
            Code::E303 => "undefined variable",
            Code::E304 => "return type mismatch",
            Code::E305 => "implicit Float/FastFloat mix",
            Code::E402 => "schema mismatch in typed file import",
            Code::E501 => "missing publisher token",
            Code::S101 => "capability mismatch",
            Code::S102 => "permission ceiling exceeded",
            Code::S201 => "untrusted path mutation",
            Code::S301 => "protected path violation",
            Code::S401 => "unapproved child process execution",
            Code::S501 => "capability analysis timeout",
            Code::W104 => "self-capturing closure",
            Code::W108 => "mutual strong fields",
            Code::W109 => "allowed cycle never cleared",
            Code::W201 => "unrecognized manifest section or field",
            Code::W204 => "deprecated `pub` visibility",
        }
    }

    pub fn from_str(s: &str) -> Option<Code> {
        Some(match s {
            "E005" => Code::E005,
            "E105" => Code::E105,
            "E107" => Code::E107,
            "E108" => Code::E108,
            "E109" => Code::E109,
            "E110" => Code::E110,
            "E111" => Code::E111,
            "E112" => Code::E112,
            "E201" => Code::E201,
            "E202" => Code::E202,
            "E203" => Code::E203,
            "E204" => Code::E204,
            "E205" => Code::E205,
            "E206" => Code::E206,
            "E302" => Code::E302,
            "E303" => Code::E303,
            "E304" => Code::E304,
            "E305" => Code::E305,
            "E402" => Code::E402,
            "E501" => Code::E501,
            "S101" => Code::S101,
            "S102" => Code::S102,
            "S201" => Code::S201,
            "S301" => Code::S301,
            "S401" => Code::S401,
            "S501" => Code::S501,
            "W104" => Code::W104,
            "W108" => Code::W108,
            "W109" => Code::W109,
            "W201" => Code::W201,
            "W204" => Code::W204,
            _ => return None,
        })
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

pub const ALL: &[Code] = &[
    Code::E005,
    Code::E105,
    Code::E107,
    Code::E108,
    Code::E109,
    Code::E110,
    Code::E111,
    Code::E112,
    Code::E201,
    Code::E202,
    Code::E203,
    Code::E204,
    Code::E205,
    Code::E206,
    Code::E302,
    Code::E303,
    Code::E304,
    Code::E305,
    Code::E402,
    Code::E501,
    Code::S101,
    Code::S102,
    Code::S201,
    Code::S301,
    Code::S401,
    Code::S501,
    Code::W104,
    Code::W108,
    Code::W109,
    Code::W201,
    Code::W204,
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: Code,
    pub message: String,
    pub span: Option<Span>,
    pub hint: Option<String>,
    pub file: Option<std::path::PathBuf>,
}

impl Diagnostic {
    pub fn new(code: Code, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            code,
            message: message.into(),
            span: None,
            hint: None,
            file: None,
        }
    }

    pub fn with_span(mut self, span: Span) -> Diagnostic {
        self.span = Some(span);
        self
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Diagnostic {
        self.hint = Some(hint.into());
        self
    }

    pub fn with_file(mut self, file: std::path::PathBuf) -> Diagnostic {
        self.file = Some(file);
        self
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = if self.code.is_warning() {
            "warning"
        } else {
            "error"
        };
        write!(f, "{}[{}]: {}", kind, self.code, self.message)?;
        if let Some(span) = self.span {
            write!(f, " ({}..{})", span.start, span.end)?;
        }
        if let Some(hint) = &self.hint {
            write!(f, " hint: {}", hint)?;
        }
        Ok(())
    }
}

pub fn line_col(src: &str, offset: u32) -> (usize, usize) {
    let off = (offset as usize).min(src.len());
    let head = &src[..off];
    let line = head.bytes().filter(|&b| b == b'\n').count() + 1;
    let col = off - head.rfind('\n').map(|i| i + 1).unwrap_or(0) + 1;
    (line, col)
}

pub type DeclFiles = std::collections::BTreeMap<(u32, u32), std::path::PathBuf>;

pub fn decl_file(files: &DeclFiles, span: Span) -> Option<std::path::PathBuf> {
    files.get(&(span.start, span.end)).cloned()
}

pub fn tag_new(diags: &mut Vec<Diagnostic>, from: usize, file: Option<std::path::PathBuf>) {
    if let Some(f) = file {
        for d in &mut diags[from..] {
            if d.file.is_none() {
                d.file = Some(f.clone());
            }
        }
    }
}

pub fn escape_json(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            _ => out.push(c),
        }
    }
    out
}

pub struct JsonDiagnostic {
    pub file: String,
    pub line: usize,
    pub col: usize,
    pub code: String,
    pub severity: String,
    pub message: String,
}

impl JsonDiagnostic {
    pub fn to_json(&self) -> String {
        format!(
            "{{\"file\":\"{}\",\"line\":{},\"col\":{},\"code\":\"{}\",\"severity\":\"{}\",\"message\":\"{}\"}}",
            escape_json(&self.file),
            self.line,
            self.col,
            escape_json(&self.code),
            escape_json(&self.severity),
            escape_json(&self.message),
        )
    }
}

pub fn diagnostics_to_json(items: &[JsonDiagnostic]) -> String {
    let mut out = String::from("[");
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&item.to_json());
    }
    out.push(']');
    out
}
