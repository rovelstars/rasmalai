
pub(super) fn run_fetch(package: Option<String>) {
            match cli::fetch_scope(package.as_deref()) {
                Ok(fetched) => {
                    if fetched.is_empty() {
                        println!("No git dependencies to fetch");
                    }
                    for (name, _, rev) in &fetched {
                        println!("Fetched {name} ({rev})");
                    }
                }
                Err(e) => {
                    println!("{e}");
                    std::process::exit(1);
                }
            }
            match fetch_registry_deps(package.as_deref()) {
                Ok(fetched) => {
                    for (name, version) in &fetched {
                        println!("Fetched {name}@{version}");
                    }
                }
                Err(e) => {
                    println!("{e}");
                    std::process::exit(1);
                }
            }
}

fn fetch_registry_deps(package: Option<&str>) -> Result<Vec<(String, String)>, diagnostics::Diagnostic> {            let scope_root = match package {
                Some(name) => cli::resolve_scope_target(None, Some(name))?
                    .scope_root
                    .ok_or_else(|| {
                        diagnostics::Diagnostic::new(
                            diagnostics::Code::E108,
                            "No file specified and no Project.config found",
                        )
                    })?,
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
                    })?
                }
            };
            frontend::fetch::fetch_all_registry_deps(&scope_root)
}

struct UreqTransport {
    agent: ureq::Agent,
}

impl UreqTransport {
    fn new() -> UreqTransport {
        let config = ureq::config::Config::builder()
            .timeout_global(Some(std::time::Duration::from_secs(30)))
            .http_status_as_error(false)
            .build();
        UreqTransport { agent: config.into() }
    }

    fn reply(
        what: &str,
        result: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    ) -> Result<frontend::fetch::RegistryReply, diagnostics::Diagnostic> {
        let mut response = result.map_err(|e| match e {
            ureq::Error::Timeout(_) => diagnostics::Diagnostic::new(
                diagnostics::Code::E108,
                format!("{what} timed out after 30s"),
            ),
            other => diagnostics::Diagnostic::new(
                diagnostics::Code::E108,
                format!("{what} failed: {other}"),
            ),
        })?;
        let status = response.status().as_u16();
        let spec_header = response
            .headers()
            .get("rnx-registry-spec")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let body = response.body_mut().read_to_vec().map_err(|e| {
            diagnostics::Diagnostic::new(
                diagnostics::Code::E108,
                format!("{what} failed: {e}"),
            )
        })?;
        Ok(frontend::fetch::RegistryReply { status, spec_header, body })
    }
}

impl frontend::fetch::RegistryTransport for UreqTransport {
    fn get(&self, url: &str) -> Result<frontend::fetch::RegistryReply, diagnostics::Diagnostic> {
        let req = self.agent.get(url).header("Accept", "application/json");
        let req = if frontend::fetch::origin_direct() {
            req.header("Cache-Control", "no-cache").header("Pragma", "no-cache")
        } else {
            req
        };
        Self::reply("registry request", req.call())
    }

    fn post_json(
        &self,
        url: &str,
        body: &str,
    ) -> Result<frontend::fetch::RegistryReply, diagnostics::Diagnostic> {
        let req = self
            .agent
            .post(url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json");
        let req = if frontend::fetch::origin_direct() {
            req.header("Cache-Control", "no-cache").header("Pragma", "no-cache")
        } else {
            req
        };
        Self::reply("registry request", req.send(body.as_bytes()))
    }
}

pub(crate) fn install_registry_transport() {
    frontend::fetch::set_registry_transport(std::sync::Arc::new(UreqTransport::new()));
}
