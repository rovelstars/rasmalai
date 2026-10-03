use crate::ast as A;
use crate::lsp_nav::{position_to_offset, render_params, render_type};
use diagnostics::escape_json;

pub struct CompletionItem {
    pub label: String,
    pub kind: u32,
    pub detail: String,
    pub insert_text: String,
}

fn item(label: &str, kind: u32, detail: &str) -> CompletionItem {
    CompletionItem {
        label: label.to_string(),
        kind,
        detail: detail.to_string(),
        insert_text: label.to_string(),
    }
}

fn receiver_before_dot(line: &str, col: usize) -> Option<String> {
    let bytes = line.as_bytes();
    if col == 0 || col > bytes.len() || bytes[col - 1] != b'.' {
        return None;
    }
    let mut s = col - 1;
    while s > 0 && (bytes[s - 1].is_ascii_alphanumeric() || bytes[s - 1] == b'_') {
        s -= 1;
    }
    if s == col - 1 {
        return None;
    }
    Some(line[s..col - 1].to_string())
}

fn enclosing_fn(module: &A::Module, offset: usize) -> Option<&A::FnDecl> {
    let offset = offset as u32;
    for decl in &module.decls {
        let contains = decl.span.start <= offset && offset <= decl.span.end;
        match &decl.node {
            A::Decl::Fn(func) if contains => return Some(func),
            A::Decl::Class { members, .. } | A::Decl::Struct { members, .. } | A::Decl::Trait { members, .. } => {
                for member in members {
                    if outside(member.span, offset) {
                        continue;
                    }
                    if let A::ClassMember::Method(func) = &member.node {
                        return Some(func);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

fn outside(span: diagnostics::Span, offset: u32) -> bool {
    offset < span.start || offset > span.end
}

fn collect_vars(block: &A::Block, out: &mut Vec<(String, Option<String>)>) {
    for stmt in &block.stmts {
        collect_stmt_vars(stmt, out);
    }
}

fn collect_stmt_vars(stmt: &A::Spanned<A::Stmt>, out: &mut Vec<(String, Option<String>)>) {
    match &stmt.node {
        A::Stmt::Var { name, ty, value, .. } => {
            let ty = ty.as_ref().map(|t| t.path.join(".")).or_else(|| match &value.node {
                A::Expr::Array(_) => Some("Array".to_string()),
                A::Expr::Int(_) => Some("Int".to_string()),
                A::Expr::Float(_) => Some("Float".to_string()),
                A::Expr::Bool(_) => Some("Bool".to_string()),
                A::Expr::Interp(_) => Some("String".to_string()),
                _ => None,
            });
            out.push((name.clone(), ty));
        }
        A::Stmt::If { then, otherwise, .. } => {
            collect_vars(then, out);
            if let Some(A::Else::Block(block)) = otherwise {
                collect_vars(block, out);
            }
        }
        A::Stmt::For { body, .. } | A::Stmt::While { body, .. } | A::Stmt::DoWhile { body, .. } => {
            collect_vars(body, out)
        }
        A::Stmt::Switch { cases, default, .. } => {
            for case in cases {
                for inner in &case.body {
                    collect_stmt_vars(inner, out);
                }
            }
            if let Some(stmts) = default {
                for inner in stmts {
                    collect_stmt_vars(inner, out);
                }
            }
        }
        A::Stmt::Defer(block) | A::Stmt::UnsafeBlock(block) => collect_vars(block, out),
        A::Stmt::Try { body, catch, finally } => {
            collect_vars(body, out);
            if let Some((_, block)) = catch {
                collect_vars(block, out);
            }
            if let Some(block) = finally {
                collect_vars(block, out);
            }
        }
        _ => {}
    }
}

fn receiver_type(module: &A::Module, offset: usize, name: &str) -> Option<String> {
    let func = enclosing_fn(module, offset)?;
    for param in &func.params {
        if param.name == name {
            return param.ty.as_ref().map(|t| t.path.join("."));
        }
    }
    let mut vars = Vec::new();
    if let A::FnBody::Block(block) = &func.body {
        collect_vars(block, &mut vars);
    }
    vars.into_iter().find(|(n, _)| n == name).and_then(|(_, ty)| ty)
}

fn member_items(module: &A::Module, offset: usize, receiver: &str) -> Vec<CompletionItem> {
    let ty = match receiver_type(module, offset, receiver) {
        Some(ty) => ty,
        None => return Vec::new(),
    };
    match ty.as_str() {
        "Vec4f" => ["x", "y", "z", "w"]
            .iter()
            .map(|f| item(f, 5, "Float"))
            .collect(),
        "Vec4i" => ["x", "y", "z", "w"]
            .iter()
            .map(|f| item(f, 5, "Int"))
            .collect(),
        "Array" => vec![
            item("push", 2, "push(value)"),
            item("pop", 2, "pop()"),
            item("len", 10, "Int"),
        ],
        other => {
            let mut out = Vec::new();
            for decl in &module.decls {
                let (name, members) = match &decl.node {
                    A::Decl::Class { name, members, .. } => (name, members),
                    A::Decl::Struct { name, members, .. } => (name, members),
                    _ => continue,
                };
                if name != other {
                    continue;
                }
                for member in members {
                    match &member.node {
                        A::ClassMember::Field(field) => {
                            let detail = field.ty.as_ref().map(render_type).unwrap_or_default();
                            out.push(item(&field.name, 5, &detail));
                        }
                        A::ClassMember::Method(func) => {
                            out.push(item(&func.name, 2, &format!("fn {}({})", func.name, render_params(&func.params))));
                        }
                        _ => {}
                    }
                }
            }
            out
        }
    }
}

fn bare_items(module: &A::Module, offset: usize) -> Vec<CompletionItem> {
    let mut out = Vec::new();
    if let Some(func) = enclosing_fn(module, offset) {
        for param in &func.params {
            let detail = param.ty.as_ref().map(render_type).unwrap_or_else(|| "parameter".to_string());
            out.push(item(&param.name, 6, &detail));
        }
        let mut vars = Vec::new();
        if let A::FnBody::Block(block) = &func.body {
            collect_vars(block, &mut vars);
        }
        for (name, ty) in vars {
            out.push(item(&name, 6, ty.as_deref().unwrap_or("variable")));
        }
    }
    for decl in &module.decls {
        match &decl.node {
            A::Decl::Fn(func) => out.push(item(
                &func.name,
                3,
                &format!("fn {}({})", func.name, render_params(&func.params)),
            )),
            A::Decl::Class { name, .. } => out.push(item(name, 7, "class")),
            A::Decl::Struct { name, .. } => out.push(item(name, 22, "struct")),
            A::Decl::Trait { name, .. } => out.push(item(name, 8, "trait")),
            A::Decl::Enum { name, .. } => out.push(item(name, 13, "enum")),
            A::Decl::Const { name, .. } => out.push(item(name, 21, "const")),
            _ => {}
        }
    }
    for keyword in [
        "fn", "pub", "export", "let", "const", "if", "else", "while", "for", "in", "return", "break",
        "continue", "class", "struct", "trait", "enum", "import", "from", "test", "bench",
        "switch", "case", "default", "try", "catch", "throw", "defer", "async", "await",
    ] {
        out.push(item(keyword, 14, "keyword"));
    }
    for ty in ["Int", "Float", "Bool", "Void", "String", "Vec4f", "Vec4i", "Array"] {
        out.push(item(ty, 7, "type"));
    }
    let mut seen = std::collections::HashSet::new();
    out.into_iter()
        .filter(|i| seen.insert(i.label.clone()))
        .collect()
}

pub fn get_completions(
    module: &A::Module,
    src: &str,
    line_text: &str,
    line: usize,
    col: usize,
) -> Vec<CompletionItem> {
    let offset = position_to_offset(src, line, col).unwrap_or(src.len());
    if let Some(receiver) = receiver_before_dot(line_text, col) {
        return member_items(module, offset, &receiver);
    }
    bare_items(module, offset)
}

pub fn completions_json(items: &[CompletionItem]) -> String {
    let mut out = String::from("{\"isIncomplete\":false,\"items\":[");
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{{\"label\":\"{}\",\"kind\":{},\"detail\":\"{}\",\"insertText\":\"{}\"}}",
            escape_json(&item.label),
            item.kind,
            escape_json(&item.detail),
            escape_json(&item.insert_text),
        ));
    }
    out.push_str("]}");
    out
}
