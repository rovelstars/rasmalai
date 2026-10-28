use crate::ast::Decl;
use std::collections::BTreeSet;
use std::sync::{Mutex, OnceLock};

static BASE: OnceLock<BTreeSet<String>> = OnceLock::new();
static OVERLAY: Mutex<(Option<Vec<u8>>, BTreeSet<String>)> = Mutex::new((None, BTreeSet::new()));

fn load_names(src: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
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
    names
}

fn base_catalog() -> &'static BTreeSet<String> {
    BASE.get_or_init(|| {
        crate::stdvfs::std_source_no_overlay("prelude")
            .map(|src| load_names(&src))
            .unwrap_or_default()
    })
}

fn overlay_catalog() -> Option<BTreeSet<String>> {
    let root = crate::sysroot::active_root()?;
    let digest = sysroot::Sysroot::custom(root).std_overlay_digest()?;
    let src = crate::sysroot::active_sysroot().std_source("prelude")?;
    let mut guard = OVERLAY.lock().unwrap_or_else(|e| e.into_inner());
    if guard.0.as_ref() != Some(&digest) {
        *guard = (Some(digest), load_names(&src));
    }
    Some(guard.1.clone())
}

fn contains(names: &BTreeSet<String>, name: &str) -> bool {
    if names.contains(name) {
        return true;
    }
    match name.rsplit('.').next() {
        Some(short) => names.contains(short),
        None => false,
    }
}

pub fn provides(name: &str) -> bool {
    if contains(base_catalog(), name) {
        return true;
    }
    overlay_catalog().is_some_and(|names| contains(&names, name))
}

pub fn short_name(name: &str) -> &str {
    let base = match name.find('<') {
        Some(ix) => &name[..ix],
        None => name,
    };
    base.rsplit('.').next().unwrap_or(base)
}

pub fn symbol_count() -> usize {
    let mut names = base_catalog().clone();
    if let Some(overlay) = overlay_catalog() {
        names.extend(overlay);
    }
    names.len()
}
