pub fn generate_stdlib_docs_json(
    out_dir: &std::path::Path,
) -> Result<std::path::PathBuf, diagnostics::Diagnostic> {
    let mut all = Vec::new();
    for name in stdlib::MODULES {
        let src = frontend::modules::load_std_module_source(name)?;
        let module = frontend::parser::Parser::parse_module(&src).map_err(|mut e| {
            e.message = format!("@std/{name}: {}", e.message);
            e
        })?;
        all.push(frontend::doc::collect_module(name, &module, true));
    }
    all.sort_by(|a, b| a.name.cmp(&b.name));
    std::fs::create_dir_all(out_dir).map_err(|e| {
        diagnostics::Diagnostic::new(
            diagnostics::Code::E108,
            format!("cannot write {}: {e}", out_dir.display()),
        )
    })?;
    let out_path = out_dir.join("api.json");
    std::fs::write(&out_path, frontend::doc::modules_to_json(&all)).map_err(|e| {
        diagnostics::Diagnostic::new(
            diagnostics::Code::E108,
            format!("cannot write {}: {e}", out_path.display()),
        )
    })?;
    Ok(out_path)
}
