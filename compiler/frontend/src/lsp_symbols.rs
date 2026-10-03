use crate::ast as A;
use crate::lsp_nav::{decl_name_span, next_ident, offset_to_position};
use diagnostics::{escape_json, Span};

pub struct DocumentSymbol {
    pub name: String,
    pub kind: u32,
    pub detail: Option<String>,
    pub range: Span,
    pub selection: Span,
    pub children: Vec<DocumentSymbol>,
}

fn leaf(name: &str, kind: u32, range: Span, selection: Span) -> DocumentSymbol {
    DocumentSymbol {
        name: name.to_string(),
        kind,
        detail: None,
        range,
        selection,
        children: Vec::new(),
    }
}

fn method_symbols(src: &str, members: &[A::Spanned<A::ClassMember>]) -> Vec<DocumentSymbol> {
    let mut out = Vec::new();
    for member in members {
        match &member.node {
            A::ClassMember::Method(func) => {
                let selection = decl_name_span(src, member.span.start as usize, &func.name)
                    .unwrap_or(member.span);
                out.push(leaf(&func.name, 6, member.span, selection));
            }
            A::ClassMember::Field(field) => {
                let selection = decl_name_span(src, member.span.start as usize, &field.name)
                    .unwrap_or(member.span);
                out.push(leaf(&field.name, 8, member.span, selection));
            }
            _ => {}
        }
    }
    out
}

fn build_document_symbols(module: &A::Module, src: &str) -> Vec<DocumentSymbol> {
    let mut out = Vec::new();
    for decl in &module.decls {
        match &decl.node {
            A::Decl::Fn(func) => {
                let selection = decl_name_span(src, decl.span.start as usize, &func.name)
                    .unwrap_or(decl.span);
                let mut symbol = leaf(&func.name, 12, decl.span, selection);
                if func.is_test {
                    symbol.detail = Some("test".to_string());
                } else if func.is_bench {
                    symbol.detail = Some("bench".to_string());
                }
                out.push(symbol);
            }
            A::Decl::Class { name, members, .. } => {
                let selection =
                    decl_name_span(src, decl.span.start as usize, name).unwrap_or(decl.span);
                let mut symbol = leaf(name, 5, decl.span, selection);
                symbol.children = method_symbols(src, members);
                out.push(symbol);
            }
            A::Decl::Struct { name, members, .. } => {
                let selection =
                    decl_name_span(src, decl.span.start as usize, name).unwrap_or(decl.span);
                let mut symbol = leaf(name, 23, decl.span, selection);
                symbol.children = method_symbols(src, members);
                out.push(symbol);
            }
            A::Decl::Record { name, fields, .. } => {
                let selection =
                    decl_name_span(src, decl.span.start as usize, name).unwrap_or(decl.span);
                let mut symbol = leaf(name, 23, decl.span, selection);
                let mut cursor = (decl.span.start as usize).min(src.len());
                for field in fields {
                    if let Some((_, span)) = next_field_name(src, &mut cursor, &field.name, decl.span.end as usize) {
                        symbol.children.push(leaf(&field.name, 8, span, span));
                    }
                }
                out.push(symbol);
            }
            A::Decl::Trait { name, members, .. } => {
                let selection =
                    decl_name_span(src, decl.span.start as usize, name).unwrap_or(decl.span);
                let mut symbol = leaf(name, 11, decl.span, selection);
                for member in members {
                    if let A::ClassMember::Method(func) = &member.node {
                        let selection = decl_name_span(src, member.span.start as usize, &func.name)
                            .unwrap_or(member.span);
                        symbol.children.push(leaf(&func.name, 6, member.span, selection));
                    }
                }
                out.push(symbol);
            }
            A::Decl::Interface { name, members, .. } => {
                let selection =
                    decl_name_span(src, decl.span.start as usize, name).unwrap_or(decl.span);
                let mut symbol = leaf(name, 11, decl.span, selection);
                for member in members {
                    if let A::ClassMember::Method(func) = &member.node {
                        let selection = decl_name_span(src, member.span.start as usize, &func.name)
                            .unwrap_or(member.span);
                        symbol.children.push(leaf(&func.name, 6, member.span, selection));
                    }
                }
                out.push(symbol);
            }
            A::Decl::Extension { target: _, members, .. } => {
                let mut symbol = leaf("extension", 11, decl.span, decl.span);
                for member in members {
                    if let A::ClassMember::Method(func) = &member.node {
                        let selection = decl_name_span(src, member.span.start as usize, &func.name)
                            .unwrap_or(member.span);
                        symbol.children.push(leaf(&func.name, 6, member.span, selection));
                    }
                }
                out.push(symbol);
            }
            A::Decl::Enum { name, members, .. } => {
                let selection =
                    decl_name_span(src, decl.span.start as usize, name).unwrap_or(decl.span);
                let mut symbol = leaf(name, 10, decl.span, selection);
                let mut cursor = (decl.span.start as usize).min(src.len());
                for member in members {
                    if let Some((_, span)) = next_field_name(src, &mut cursor, &member.name, decl.span.end as usize) {
                        symbol.children.push(leaf(&member.name, 22, span, span));
                    }
                }
                out.push(symbol);
            }
            A::Decl::Const { name, .. } => {
                let selection =
                    decl_name_span(src, decl.span.start as usize, name).unwrap_or(decl.span);
                out.push(leaf(name, 14, decl.span, selection));
            }
            A::Decl::Import(..)
            | A::Decl::ExportFrom(..)
            | A::Decl::ExportList(..)
            | A::Decl::ExportDefault(..)
            | A::Decl::Stmt { .. } => {}
        }
    }
    out
}

fn next_field_name(src: &str, cursor: &mut usize, name: &str, end: usize) -> Option<(String, Span)> {
    while *cursor < end {
        match next_ident(src, *cursor) {
            Some((word, span)) if (span.end as usize) <= end => {
                *cursor = span.end as usize;
                if word == name {
                    return Some((word, span));
                }
            }
            _ => break,
        }
    }
    None
}

fn range_json(src: &str, span: Span) -> String {
    let (sl, sc) = offset_to_position(src, span.start as usize);
    let (el, ec) = offset_to_position(src, span.end as usize);
    format!(
        "{{\"start\":{{\"line\":{sl},\"character\":{sc}}},\"end\":{{\"line\":{el},\"character\":{ec}}}}}"
    )
}

fn symbol_json(src: &str, symbol: &DocumentSymbol) -> String {
    let mut out = format!(
        "{{\"name\":\"{}\",\"kind\":{}",
        escape_json(&symbol.name),
        symbol.kind
    );
    match &symbol.detail {
        Some(detail) => out.push_str(&format!(",\"detail\":\"{}\"", escape_json(detail))),
        None => out.push_str(",\"detail\":null"),
    }
    out.push_str(&format!(
        ",\"range\":{},\"selectionRange\":{}",
        range_json(src, symbol.range),
        range_json(src, symbol.selection)
    ));
    out.push_str(",\"children\":[");
    for (i, child) in symbol.children.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&symbol_json(src, child));
    }
    out.push_str("]}");
    out
}

pub fn symbols_json(module: &A::Module, src: &str) -> String {
    let symbols = build_document_symbols(module, src);
    let mut out = String::from("[");
    for (i, symbol) in symbols.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&symbol_json(src, symbol));
    }
    out.push(']');
    out
}
