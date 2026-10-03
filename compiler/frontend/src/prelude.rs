use crate::ast::Decl;
use std::collections::BTreeSet;
use std::sync::OnceLock;

static CATALOG: OnceLock<BTreeSet<String>> = OnceLock::new();

fn load_catalog() -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    if let Some(src) = stdlib::source("prelude") {
        if let Ok(module) = crate::parser::Parser::parse_module(src) {
            for decl in &module.decls {
                match &decl.node {
                    Decl::Fn(f) => {
                        names.insert(f.name.clone());
                    }
                    Decl::Class { name, .. }
                    | Decl::Struct { name, .. }
                    | Decl::Enum { name, .. }
                    | Decl::Record { name, .. } => {
                        names.insert(name.clone());
                    }
                    Decl::Const { name, .. } => {
                        names.insert(name.clone());
                    }
                    _ => {}
                }
            }
        }
    }
    names
}

fn catalog() -> &'static BTreeSet<String> {
    CATALOG.get_or_init(load_catalog)
}

pub fn provides(name: &str) -> bool {
    if catalog().contains(name) {
        return true;
    }
    match name.rsplit('.').next() {
        Some(short) => catalog().contains(short),
        None => false,
    }
}

pub fn short_name(name: &str) -> &str {
    let base = match name.find('<') {
        Some(ix) => &name[..ix],
        None => name,
    };
    base.rsplit('.').next().unwrap_or(base)
}

pub fn symbol_count() -> usize {
    catalog().len()
}
