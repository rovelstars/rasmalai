use frontend::macros::MacroOptions;

/// Resolve the checkout prelude and expand call macros before lowering.
///
/// The macro bodies live in `@std/io`, so tests that lower a source string
/// need the real stdlib graph; parsing the string alone leaves `println!`
/// (and any other macro) as an unexpanded call.
pub fn resolve_for_lowering(src: &str) -> frontend::ast::Module {
    let mut module =
        frontend::modules::ModuleGraph::from_source(src).unwrap_or_else(|e| panic!("{e}"));
    let base = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let mut opts = MacroOptions::new(base);
    let diags = frontend::macros::expand_syntax(&mut module, &mut opts);
    assert!(diags.iter().all(|d| d.code.is_warning()), "{diags:?}");
    let diags = frontend::macros::expand_comptime(&mut module, &mut opts, None);
    assert!(diags.iter().all(|d| d.code.is_warning()), "{diags:?}");
    let diags = frontend::desugar::desugar(&mut module);
    assert!(diags.is_empty(), "{diags:?}");
    module
}
