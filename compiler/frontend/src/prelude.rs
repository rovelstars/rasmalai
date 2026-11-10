use crate::ast::{Decl, ExportClause, ImportSource};
use diagnostics::{Code, Diagnostic};
use std::collections::BTreeSet;
use std::sync::Mutex;

static CATALOG: Mutex<Option<(String, BTreeSet<String>)>> = Mutex::new(None);

const REQUIRED: &[&str] = &[
    "Int",
    "Float",
    "FastFloat",
    "Bool",
    "String",
    "Char",
    "Array",
    "Map",
    "Set",
    "Date",
    "Error",
    "Any",
    "Void",
    "Null",
    "GenRef",
    "Result",
    "Iterator",
];

// `Null` is a language builtin with no declaration in the stock prelude.
const BUILTIN_NAMES: &[&str] = &["Null"];

fn collect_export_from(clause: &ExportClause, names: &mut BTreeSet<String>) {
    match clause {
        ExportClause::Named(specs) => {
            for spec in specs {
                names.insert(spec.alias.clone().unwrap_or_else(|| spec.name.clone()));
            }
        }
        ExportClause::All { alias } => {
            if let Some(alias) = alias {
                names.insert(alias.clone());
            }
            // `export * from "..."` names need the target module's export
            // table, which is built from the module graph after catalogs
            // are validated. Such names are not collected here.
        }
        ExportClause::Native(specs) => {
            for spec in specs {
                names.insert(spec.alias.clone().unwrap_or_else(|| spec.name.clone()));
            }
        }
    }
}

pub(crate) fn load_names(src: &str) -> BTreeSet<String> {
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
                | Decl::Interface { name, .. }
                | Decl::Record { name, .. } => {
                    names.insert(name.clone());
                }
                Decl::Const { name, .. } => {
                    names.insert(name.clone());
                }
                Decl::ExportFrom(d) => collect_export_from(&d.clause, &mut names),
                _ => {}
            }
        }
    }
    names
}

fn selected_source() -> (String, String) {
    if let Some((spec, src)) = crate::modules::selected_prelude() {
        return (spec, src);
    }
    let spec = crate::modules::prelude_spec(None);
    let src = crate::modules::load_prelude_source(&spec).unwrap_or_default();
    (spec, src)
}

fn catalog() -> BTreeSet<String> {
    let (spec, src) = selected_source();
    let mut guard = CATALOG.lock().unwrap_or_else(|e| e.into_inner());
    let stale = guard.as_ref().is_none_or(|(cached, _)| cached != &spec);
    if stale {
        *guard = Some((spec, load_names(&src)));
    }
    guard
        .as_ref()
        .map(|(_, names)| names.clone())
        .unwrap_or_default()
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
    contains(&catalog(), name)
}

pub fn canonical_path(name: &str) -> Option<String> {
    let (_, src) = selected_source();
    let module = crate::parser::Parser::parse_module(&src).ok()?;
    for decl in &module.decls {
        match &decl.node {
            Decl::Fn(f) if f.name == name => return Some(format!("std.prelude.{name}")),
            Decl::Class { name: n, .. }
            | Decl::Struct { name: n, .. }
            | Decl::Enum { name: n, .. }
            | Decl::Interface { name: n, .. }
            | Decl::Record { name: n, .. }
            | Decl::Trait { name: n, .. } => {
                if n == name {
                    return Some(format!("std.prelude.{name}"));
                }
            }
            Decl::Const { name: n, .. } if n == name => {
                return Some(format!("std.prelude.{name}"));
            }
            Decl::ExportFrom(d) => {
                if let ImportSource::Module(source) = &d.source
                    && let ExportClause::Named(specs) = &d.clause
                {
                    for spec in specs {
                        if spec.alias.clone().unwrap_or_else(|| spec.name.clone()) == name {
                            if crate::modules::is_std_spec(source) {
                                let rest = crate::modules::std_rest(source);
                                if rest == "foundation" {
                                    // Foundation declarations are merged under
                                    // the prelude namespace for compatibility.
                                    return Some(format!("std.prelude.{name}"));
                                }
                                if !rest.is_empty() {
                                    return Some(format!(
                                        "std.{}.{}",
                                        rest.replace('/', "."),
                                        spec.name
                                    ));
                                }
                            }
                            return None;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    if contains(&load_names(&src), name) {
        return Some(format!("std.prelude.{name}"));
    }
    None
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

pub fn validate_catalog(names: &BTreeSet<String>) -> Result<(), Diagnostic> {
    for name in REQUIRED {
        if names.contains(*name) || BUILTIN_NAMES.contains(name) {
            continue;
        }
        let spec = crate::modules::selected_prelude()
            .map(|(spec, _)| spec)
            .unwrap_or_else(|| crate::modules::prelude_spec(None));
        return Err(Diagnostic::new(
            Code::E108,
            format!("prelude package `{spec}` is missing required name `{name}`"),
        )
        .with_hint("declare the name in the selected prelude or choose another prelude package"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_names_follows_named_reexports() {
        let names = load_names(
            "export { Marker, Other as Renamed } from \"@std/time\";\n\
             export * as ns from \"@std/fs\";\n",
        );
        assert!(names.contains("Marker"));
        assert!(names.contains("Renamed"));
        assert!(names.contains("ns"));
        assert!(!names.contains("Other"));
    }

    #[test]
    fn validate_catalog_names_the_missing_name_and_package() {
        let mut names = BTreeSet::new();
        for name in REQUIRED {
            if *name != "Iterator" {
                names.insert(name.to_string());
            }
        }
        let err = validate_catalog(&names).unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("Iterator"), "{}", err.message);
        assert!(err.message.contains("prelude"), "{}", err.message);
    }

    #[test]
    fn stock_prelude_exports_the_moved_names() {
        let src = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../stdlib/src/prelude/lib.rnx"
        ))
        .expect("checkout prelude");
        let names = load_names(&src);
        for name in ["Promise", "RnxHost", "Date", "IO", "Thread", "ThreadPool"] {
            assert!(contains(&names, name), "{name} missing from the stock prelude");
        }
    }
}
