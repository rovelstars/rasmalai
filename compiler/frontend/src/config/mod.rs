pub mod eval;

pub use eval::{ConfigContext, ConfigTarget, ConfigValue, eval_expr, eval_module};

use crate::parser::Parser;

pub fn parse(text: &str) -> Result<ConfigValue, String> {
    parse_with(text, ConfigTarget::host())
}

pub fn parse_with(text: &str, target: ConfigTarget) -> Result<ConfigValue, String> {
    let module = Parser::parse_module(text).map_err(|e| e.message.clone())?;
    eval_module(&module, target).map_err(|e| e.message.clone())
}

pub fn write(value: &ConfigValue) -> String {
    let mut out = String::from("export default ");
    write_value(value, 0, &mut out);
    out.push('\n');
    out
}

fn write_value(value: &ConfigValue, indent: usize, out: &mut String) {
    match value {
        ConfigValue::String(s) => write_string(s, out),
        ConfigValue::Int(n) => out.push_str(&n.to_string()),
        ConfigValue::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        ConfigValue::Object(fields) => write_object(fields, indent, out),
        ConfigValue::Array(items) => write_array(items, indent, out),
    }
}

fn write_object(fields: &[(String, ConfigValue)], indent: usize, out: &mut String) {
    if fields.is_empty() {
        out.push_str("{}");
        return;
    }
    out.push_str("{\n");
    for (key, value) in fields {
        push_indent(indent + 4, out);
        if bare_key(key) {
            out.push_str(key);
        } else {
            write_string(key, out);
        }
        out.push_str(": ");
        write_value(value, indent + 4, out);
        out.push_str(",\n");
    }
    push_indent(indent, out);
    out.push('}');
}

fn write_array(items: &[ConfigValue], indent: usize, out: &mut String) {
    if items.is_empty() {
        out.push_str("[]");
        return;
    }
    let inline = items.iter().all(|item| {
        matches!(
            item,
            ConfigValue::String(_) | ConfigValue::Int(_) | ConfigValue::Bool(_)
        )
    });
    if inline {
        out.push('[');
        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            write_value(item, indent, out);
        }
        out.push(']');
        return;
    }
    out.push_str("[\n");
    for item in items {
        push_indent(indent + 4, out);
        write_value(item, indent + 4, out);
        out.push_str(",\n");
    }
    push_indent(indent, out);
    out.push(']');
}

fn push_indent(indent: usize, out: &mut String) {
    for _ in 0..indent {
        out.push(' ');
    }
}

fn write_string(value: &str, out: &mut String) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '{' => out.push_str("\\{"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\u{{{:x}}}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn bare_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return false;
    }
    !is_keyword(key)
}

fn is_keyword(word: &str) -> bool {
    matches!(
        word,
        "true"
            | "false"
            | "null"
            | "this"
            | "super"
            | "class"
            | "struct"
            | "record"
            | "trait"
            | "interface"
            | "extension"
            | "enum"
            | "fn"
            | "init"
            | "new"
            | "deinit"
            | "onReload"
            | "extends"
            | "with"
            | "let"
            | "const"
            | "static"
            | "if"
            | "else"
            | "for"
            | "while"
            | "in"
            | "return"
            | "break"
            | "continue"
            | "try"
            | "catch"
            | "finally"
            | "throw"
            | "throws"
            | "defer"
            | "guard"
            | "switch"
            | "case"
            | "default"
            | "do"
            | "fallthrough"
            | "is"
            | "import"
            | "export"
            | "from"
            | "as"
            | "public"
            | "pub"
            | "private"
            | "unsafe"
            | "comptime"
            | "native"
            | "runtime"
            | "async"
            | "await"
            | "pass"
    )
}
