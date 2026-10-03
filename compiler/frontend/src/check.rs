use crate::desugar::desugar;
use crate::modules::ModuleGraph;
use crate::parser::Parser;
use crate::semantic;
use diagnostics::Diagnostic;
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
    split(diags)
}

pub fn check_package(root: &Path) -> Result<Vec<Diagnostic>, Vec<Diagnostic>> {
    use crate::modules::merged_decl_files;
    let graph = match ModuleGraph::build_collecting(root) {
        Ok(g) => g,
        Err(errs) => return Err(errs),
    };
    let files = merged_decl_files(&graph);
    let mut module = match graph.resolve() {
        Ok(m) => m,
        Err(e) => return Err(vec![e]),
    };
    let mut diags = module.warnings.clone();
    diags.extend(crate::desugar::desugar_with_files(&mut module, &files));
    diags.extend(semantic::check_with_files(&module, &files));
    if diags.iter().all(|d| d.code.is_warning()) {
        diags.extend(graph.isolation_errors());
    }
    split(diags)
}
