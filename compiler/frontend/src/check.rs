use crate::desugar::desugar;
use crate::modules::ModuleGraph;
use crate::parser::Parser;
use crate::semantic;
use diagnostics::{Code, Diagnostic};
use std::path::Path;

fn sort_diags(diags: &mut [Diagnostic]) {
    diags.sort_by(|a, b| {
        a.code
            .as_str()
            .cmp(b.code.as_str())
            .then(a.message.cmp(&b.message))
    });
}

fn split(diags: Vec<Diagnostic>) -> Result<Vec<Diagnostic>, Vec<Diagnostic>> {
    let (warnings, errors) = partition(diags);
    if errors.is_empty() {
        Ok(warnings)
    } else {
        Err(errors)
    }
}

pub fn check_source_all(src: &str) -> (Vec<Diagnostic>, Vec<Diagnostic>) {
    let mut module = match Parser::parse_module_all(src) {
        Ok(m) => m,
        Err(errs) => return (Vec::new(), errs),
    };
    let mut diags = module.warnings.clone();
    diags.extend(desugar(&mut module));
    diags.extend(semantic::check(&module));
    for r in crate::security::collect_runtime_imports(&module) {
        diags.push(crate::security::runtime_frame_rejection(
            &r.module, &r.symbol, r.span,
        ));
    }
    partition(diags)
}

fn partition(diags: Vec<Diagnostic>) -> (Vec<Diagnostic>, Vec<Diagnostic>) {
    let mut warnings = Vec::new();
    let mut errors = Vec::new();
    for d in diags {
        if d.code.is_warning() {
            warnings.push(d);
        } else {
            errors.push(d);
        }
    }
    sort_diags(&mut warnings);
    sort_diags(&mut errors);
    (warnings, errors)
}

pub fn check_source(src: &str) -> Result<Vec<Diagnostic>, Vec<Diagnostic>> {
    let mut module = match Parser::parse_module(src) {
        Ok(m) => m,
        Err(e) => return Err(vec![e]),
    };
    let mut diags = module.warnings.clone();
    diags.extend(desugar(&mut module));
    diags.extend(semantic::check(&module));
    for r in crate::security::collect_runtime_imports(&module) {
        diags.push(crate::security::runtime_frame_rejection(
            &r.module, &r.symbol, r.span,
        ));
    }
    split(diags)
}

pub fn check_package(root: &Path) -> Result<Vec<Diagnostic>, Vec<Diagnostic>> {
    check_package_with_dump(root, None)
}

fn same_file(a: &Path, b: &Path) -> bool {
    let ca = std::fs::canonicalize(a).unwrap_or_else(|_| a.to_path_buf());
    let cb = std::fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
    ca == cb
}

fn cabi_surface(root: &Path) -> Option<std::path::PathBuf> {
    let dir = crate::project::find_project_root(root)?;
    let cfg = crate::project::ProjectConfig::load_from_dir(&dir).ok().flatten()?;
    let path = cfg.cabi_path(&dir)?;
    path.is_file().then_some(path)
}

pub fn check_package_with_dump(
    root: &Path,
    dump: Option<std::path::PathBuf>,
) -> Result<Vec<Diagnostic>, Vec<Diagnostic>> {
    use crate::modules::merged_decl_files;
    let cabi = cabi_surface(root);
    let graph = match cabi.as_deref() {
        Some(surface) => ModuleGraph::build_collecting_extra(root, &[surface.to_path_buf()]),
        None => ModuleGraph::build_collecting(root),
    };
    let graph = match graph {
        Ok(g) => g,
        Err(errs) => return Err(errs),
    };
    let files = merged_decl_files(&graph);
    let mut module = match graph.resolve() {
        Ok(m) => m,
        Err(e) => return Err(vec![e]),
    };
    let base = root
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let mut opts = crate::macros::MacroOptions::new(base);
    opts.dump_dir = dump;
    let mut diags = module.warnings.clone();
    diags.extend(crate::macros::expand_syntax(&mut module, &mut opts));
    if diags.iter().any(|d| !d.code.is_warning()) {
        return split(diags);
    }
    diags.extend(crate::macros::expand_comptime(&mut module, &mut opts, None));
    let mut surface_misuse = false;
    if let Some(surface) = cabi.as_deref() {
        for f in &graph.files {
            if !same_file(&f.path, surface) {
                continue;
            }
            for decl in &f.module.decls {
                let message = match &decl.node {
                    crate::ast::Decl::ExportFrom(_) => {
                        Some("an `export { ... } from` re-export writes no C symbol")
                    }
                    crate::ast::Decl::ExportList(_) => {
                        Some("an `export { ... }` list writes no C symbol")
                    }
                    crate::ast::Decl::ExportDefault(_) => {
                        Some("an `export default` writes no C symbol")
                    }
                    _ => None,
                };
                if let Some(message) = message {
                    surface_misuse = true;
                    diags.push(
                        Diagnostic::new(Code::E108, message)
                            .with_span(decl.span)
                            .with_file(f.path.clone())
                            .with_hint(
                                "declare a wrapper with a C-representable signature in the cabi file; the surface is declarations, not re-exports",
                            ),
                    );
                }
            }
        }
    }
    diags.extend(crate::desugar::desugar_with_files(&mut module, &files));
    diags.extend(semantic::check_with_files_surface_misuse(
        &module,
        &files,
        cabi.as_deref(),
        surface_misuse,
    ));
    diags.extend(crate::security::verify_runtime_imports(&graph));
    if diags.iter().all(|d| d.code.is_warning()) {
        diags.extend(graph.isolation_errors());
    }
    split(diags)
}
