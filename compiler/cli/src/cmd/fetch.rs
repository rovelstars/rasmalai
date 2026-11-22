
pub(super) fn run_fetch(package: Option<String>, quiet: bool) {
            let theme = diagnostics::theme::AuraTheme::active();
            match cli::fetch_scope(package.as_deref()) {
                Ok(fetched) => {
                    if fetched.is_empty() {
                        println!(
                            "{}",
                            theme.paint(
                                diagnostics::theme::AuraColor::Muted,
                                "No git dependencies to fetch."
                            )
                        );
                    }
                    for (name, _, rev) in &fetched {
                        println!(
                            "Fetched {} ({})",
                            theme.paint(diagnostics::theme::AuraColor::Cyan, name),
                            theme.paint(diagnostics::theme::AuraColor::Muted, rev)
                        );
                    }
                }
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
            let result = super::with_status(quiet, "Fetching dependencies", || {
                fetch_registry_deps(package.as_deref())
            });
            match result {
                Ok(fetched) => {
                    for (name, version) in &fetched {
                        println!(
                            "Fetched {}@{}",
                            theme.paint(diagnostics::theme::AuraColor::Cyan, name),
                            theme.paint(diagnostics::theme::AuraColor::Muted, version)
                        );
                    }
                }
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
            match resolve_fetch_scope(package.as_deref()) {
                Ok(scope_root) => {
                    let report = frontend::products::precompile_scope(&scope_root);
                    if report.entries > 0 {
                        println!(
                            "Precompiled {} dep products ({} entries in {} projects)",
                            theme.paint(
                                diagnostics::theme::AuraColor::Orange,
                                &report.populated.to_string()
                            ),
                            theme.paint(
                                diagnostics::theme::AuraColor::Orange,
                                &report.entries.to_string()
                            ),
                            theme.paint(
                                diagnostics::theme::AuraColor::Orange,
                                &report.projects.to_string()
                            )
                        );
                    }
                }
                Err(_) => {}
            }
}

fn resolve_fetch_scope(package: Option<&str>) -> Result<std::path::PathBuf, diagnostics::Diagnostic> {            match package {
                Some(name) => cli::resolve_scope_target(None, Some(name))?
                    .scope_root
                    .ok_or_else(|| {
                        diagnostics::Diagnostic::new(
                            diagnostics::Code::E108,
                            "No file specified and no Project.config found",
                        )
                    }),
                None => {
                    let cwd = std::env::current_dir().map_err(|e| {
                        diagnostics::Diagnostic::new(
                            diagnostics::Code::E108,
                            format!("cannot read working directory: {e}"),
                        )
                    })?;
                    frontend::project::find_workspace_root(&cwd).ok_or_else(|| {
                        diagnostics::Diagnostic::new(
                            diagnostics::Code::E108,
                            "No file specified and no Project.config found",
                        )
                    })
                }
            }
}

fn fetch_registry_deps(package: Option<&str>) -> Result<Vec<(String, String)>, diagnostics::Diagnostic> {            let scope_root = resolve_fetch_scope(package)?;
            frontend::fetch::fetch_all_registry_deps(&scope_root)
}

pub(crate) fn install_registry_transport() {
    frontend::fetch::install_http2_transport();
}
