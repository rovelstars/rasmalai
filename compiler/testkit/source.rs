//! Resolve and prepare a Rasmalai source file for lowering.
//!
//! Test harnesses that lower a source string or file must run the same
//! frontend passes as `rnx check`: merge the module graph, expand call and
//! comptime macros, then desugar. Lowering a parsed module alone leaves
//! `println!` unexpanded and turns `print(...)` into an unresolved call.
//!
//! A test target pulls this in at its crate root:
//!
//! ```ignore
//! #[cfg(test)]
//! #[path = "../../testkit/source.rs"]
//! mod source;
//! ```

use frontend::ast::Module;
use std::path::{Path, PathBuf};

#[allow(dead_code)]
pub fn resolve_file(root: &Path) -> Module {
    let graph = frontend::modules::ModuleGraph::build(root).unwrap_or_else(|e| panic!("{e}"));
    let base = root
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    finish(graph.resolve().unwrap_or_else(|e| panic!("{e}")), base)
}

#[allow(dead_code)]
pub fn resolve_source(src: &str) -> Module {
    let module = frontend::modules::ModuleGraph::from_source(src).unwrap_or_else(|e| panic!("{e}"));
    let base = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    finish(module, base)
}

pub fn finish(mut module: Module, base: PathBuf) -> Module {
    let diags = frontend::macros::expand_module(&mut module, base);
    assert!(diags.iter().all(|d| d.code.is_warning()), "{diags:?}");
    let diags = frontend::desugar::desugar(&mut module);
    assert!(diags.is_empty(), "{diags:?}");
    module
}
