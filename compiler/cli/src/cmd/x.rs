use diagnostics::{Code, Diagnostic};
use frontend::project::{PermissionDecl, ProjectConfig};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

pub(super) fn run_x(package: String, bin: Option<String>, yes: bool, args: Vec<String>) {
    let (name, range) = match parse_package_spec(&package) {
        Ok(v) => v,
        Err(e) => fail(e),
    };
    let (default, overrides) = frontend::stdlib_seed::std_registry_from_env();
    let have = frontend::fetch::scan_cache_have();
    let (dir, cfg, version) = match frontend::fetch::resolve_registry_package(
        &name,
        &range,
        default.as_ref(),
        &overrides,
        None,
        &have,
        None,
    ) {
        Ok(v) => v,
        Err(e) => fail(e),
    };
    let (bin_name, rel) = match select_entry(&cfg, bin.as_deref()) {
        Ok(v) => v,
        Err(e) => fail(e),
    };
    let entry = match canonical_entry(&dir, &rel) {
        Ok(p) => p,
        Err(e) => fail(e),
    };
    println!("Running {name}@{version} {bin_name} ({})", entry.display());
    if let Err(e) = approve_capabilities(
        &name,
        &version,
        cfg.permissions.as_deref().unwrap_or(&[]),
        yes,
    ) {
        fail(e);
    }
    let code = run_entry(&entry, &args);
    if code != 0 {
        std::process::exit(code);
    }
}

fn fail(e: Diagnostic) -> ! {
    println!("{e}");
    std::process::exit(1);
}

fn parse_package_spec(spec: &str) -> Result<(String, String), Diagnostic> {
    let sel = frontend::deplock::parse_update_selector(spec)?;
    let range = sel.selector.unwrap_or_else(|| "*".to_string());
    if !frontend::fetch::is_valid_range(&range) {
        return Err(Diagnostic::new(
            Code::E108,
            format!(
                "requirement for `{}` is not a valid range (*, latest, X.Y.Z, ^, ~, >=)",
                sel.name
            ),
        ));
    }
    Ok((sel.name, range))
}

fn bin_names(cfg: &ProjectConfig) -> String {
    let names: Vec<&str> = cfg.entries.bins.keys().map(|s| s.as_str()).collect();
    if names.is_empty() {
        "none".to_string()
    } else {
        names.join(", ")
    }
}

fn select_entry(cfg: &ProjectConfig, bin: Option<&str>) -> Result<(String, String), Diagnostic> {
    let bins = &cfg.entries.bins;
    if let Some(name) = bin {
        return match bins.get(name) {
            Some(path) => Ok((name.to_string(), path.clone())),
            None => Err(Diagnostic::new(
                Code::E108,
                format!(
                    "package `{}` has no bin `{name}` (available: {})",
                    cfg.name,
                    bin_names(cfg)
                ),
            )),
        };
    }
    if let Some((name, path)) = bins.iter().next() {
        if bins.len() == 1 {
            return Ok((name.clone(), path.clone()));
        }
        return Err(Diagnostic::new(
            Code::E108,
            format!(
                "package `{}` declares several bins ({}); pick one with `rnx x {}@{} <bin>`",
                cfg.name,
                bin_names(cfg),
                cfg.name,
                cfg.version
            ),
        ));
    }
    Ok(("main".to_string(), cfg.entries.main.clone()))
}

fn canonical_entry(dir: &Path, rel: &str) -> Result<PathBuf, Diagnostic> {
    let root = std::fs::canonicalize(dir).map_err(|e| {
        Diagnostic::new(
            Code::E108,
            format!("cannot read package directory {}: {e}", dir.display()),
        )
    })?;
    let joined = root.join(rel);
    let path = std::fs::canonicalize(&joined).map_err(|e| {
        Diagnostic::new(
            Code::E108,
            format!("package entry `{rel}` is missing ({}): {e}", joined.display()),
        )
    })?;
    if !path.starts_with(&root) {
        return Err(Diagnostic::new(
            Code::E108,
            format!("package entry `{rel}` escapes the package directory"),
        ));
    }
    Ok(path)
}

fn approve_capabilities(
    pkg: &str,
    version: &str,
    perms: &[PermissionDecl],
    yes: bool,
) -> Result<(), Diagnostic> {
    if perms.is_empty() {
        return Ok(());
    }
    println!("{pkg}@{version} declares capabilities:");
    for perm in perms {
        match perm.reason.as_deref().map(str::trim) {
            Some(reason) if !reason.is_empty() => println!("  {} ({reason})", perm.perm),
            _ => println!("  {}", perm.perm),
        }
    }
    if yes {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        return Err(Diagnostic::new(
            Code::E108,
            format!(
                "`{pkg}` declares capabilities and stdin is not a terminal; re-run with --yes to approve"
            ),
        ));
    }
    print!("Allow? [y/N] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() || !approved(&line) {
        return Err(Diagnostic::new(
            Code::E108,
            format!("capabilities for `{pkg}` were not approved; nothing was run"),
        ));
    }
    Ok(())
}

fn approved(answer: &str) -> bool {
    let answer = answer.trim();
    answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes")
}

fn run_entry(entry: &Path, args: &[String]) -> i32 {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => fail(Diagnostic::new(
            Code::E108,
            format!("cannot locate the rnx binary: {e}"),
        )),
    };
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => fail(Diagnostic::new(
            Code::E108,
            format!("cannot read working directory: {e}"),
        )),
    };
    let mut command = std::process::Command::new(exe);
    command.arg("run").arg(entry).arg("--");
    command.args(args);
    command.current_dir(cwd);
    match command.status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(e) => fail(Diagnostic::new(
            Code::E108,
            format!("cannot run {}: {e}", entry.display()),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn config_with_bins(bins: &[(&str, &str)]) -> ProjectConfig {
        let mut entries = frontend::project::Entries::default();
        entries.bins = bins
            .iter()
            .map(|(name, path)| (name.to_string(), path.to_string()))
            .collect::<BTreeMap<_, _>>();
        ProjectConfig {
            name: "tool".to_string(),
            version: "1.2.3".to_string(),
            description: String::new(),
            engine: "*".to_string(),
            entries,
            registry: None,
            registries: BTreeMap::new(),
            dependencies: BTreeMap::new(),
            permissions: None,
            keywords: Vec::new(),
            artifacts: None,
            license: None,
            native_pins: BTreeMap::new(),
            tasks: BTreeMap::new(),
            bindgen: None,
        }
    }

    #[test]
    fn spec_split_defaults_to_any_range() {
        assert_eq!(
            parse_package_spec("tool").unwrap(),
            ("tool".to_string(), "*".to_string())
        );
        assert_eq!(
            parse_package_spec("tool@1.2.3").unwrap(),
            ("tool".to_string(), "1.2.3".to_string())
        );
        assert_eq!(
            parse_package_spec("@scope/tool@^1.2.0").unwrap(),
            ("@scope/tool".to_string(), "^1.2.0".to_string())
        );
        assert_eq!(
            parse_package_spec("@scope/tool").unwrap(),
            ("@scope/tool".to_string(), "*".to_string())
        );
        assert!(parse_package_spec("").is_err());
        assert!(parse_package_spec("tool@").is_err());
    }

    #[test]
    fn no_bins_falls_back_to_main() {
        let cfg = config_with_bins(&[]);
        assert_eq!(
            select_entry(&cfg, None).unwrap(),
            ("main".to_string(), cfg.entries.main.clone())
        );
    }

    #[test]
    fn single_bin_is_implicit() {
        let cfg = config_with_bins(&[("tool", "src/tool.rnx")]);
        assert_eq!(
            select_entry(&cfg, None).unwrap(),
            ("tool".to_string(), "src/tool.rnx".to_string())
        );
    }

    #[test]
    fn several_bins_require_a_name() {
        let cfg = config_with_bins(&[("alpha", "src/alpha.rnx"), ("beta", "src/beta.rnx")]);
        let err = select_entry(&cfg, None).unwrap_err();
        assert!(err.message.contains("alpha"));
        assert!(err.message.contains("beta"));
        assert_eq!(
            select_entry(&cfg, Some("beta")).unwrap(),
            ("beta".to_string(), "src/beta.rnx".to_string())
        );
    }

    #[test]
    fn unknown_bin_lists_available_names() {
        let cfg = config_with_bins(&[("alpha", "src/alpha.rnx"), ("beta", "src/beta.rnx")]);
        let err = select_entry(&cfg, Some("gamma")).unwrap_err();
        assert!(err.message.contains("gamma"));
        assert!(err.message.contains("alpha, beta"));
    }

    #[test]
    fn approval_accepts_y_and_yes_only() {
        assert!(approved("y\n"));
        assert!(approved("YES"));
        assert!(approved(" Yes "));
        assert!(!approved(""));
        assert!(!approved("n"));
        assert!(!approved("yep"));
    }
}
