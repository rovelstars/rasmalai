use std::path::Path;

pub const SHORT: &str = env!("CARGO_PKG_VERSION");
pub const COMMIT: &str = env!("RNX_COMMIT");
pub const BUILD_TIMESTAMP: &str = env!("RNX_BUILD_TIMESTAMP");
pub const INSTALL_REASON: &str = env!("RNX_INSTALLATION_MANAGED_REASON");
pub const TARGET_TRIPLE: &str = env!("RNX_TARGET_TRIPLE");

pub fn short_line() -> String {
    format!("rnx {SHORT}")
}

pub fn install_kind_label(reason: Option<&str>) -> String {
    match reason {
        Some(r) if !r.is_empty() => format!("managed:{r}"),
        _ => "standalone".to_string(),
    }
}

pub fn display_commit(commit: &str) -> &str {
    if commit.is_empty() { "unknown" } else { commit }
}

pub fn display_built(built: &str) -> &str {
    if built.is_empty() { "unknown" } else { built }
}

pub fn stdlib_label(vendor: Option<&str>, checkout: Option<&Path>) -> String {
    if let Some(dir) = vendor.map(str::trim).filter(|dir| !dir.is_empty()) {
        return format!("{dir} (RNX_VENDOR_DIR)");
    }
    match checkout {
        Some(path) => format!("{} (source checkout; @std/* auto-vendored)", path.display()),
        None => "registry cache".to_string(),
    }
}

pub struct BuildRecord<'a> {
    pub version: &'a str,
    pub commit: &'a str,
    pub built: &'a str,
    pub install: &'a str,
    pub binary: &'a str,
    pub stdlib: &'a str,
    pub target: &'a str,
    pub llvm: &'a str,
}

pub fn render_long(r: &BuildRecord<'_>) -> String {
    format!(
        "{}\ncommit: {}\nbuilt: {}\ninstall: {}\nbinary: {}\nstdlib: {}\ntarget: {}\nllvm: {}",
        r.version,
        display_commit(r.commit),
        display_built(r.built),
        r.install,
        r.binary,
        r.stdlib,
        r.target,
        r.llvm
    )
}

pub fn long_version() -> String {
    let exe = std::env::current_exe().ok();
    let binary = exe
        .as_deref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let vendor = std::env::var(frontend::fetch::LOCAL_VENDOR_ENV).ok();
    let checkout = crate::checkout::source_stdlib();
    let stdlib = stdlib_label(vendor.as_deref(), checkout.as_deref());
    render_long(&BuildRecord {
        version: SHORT,
        commit: COMMIT,
        built: BUILD_TIMESTAMP,
        install: &install_kind_label(Some(INSTALL_REASON)),
        binary: &binary,
        stdlib: &stdlib,
        target: TARGET_TRIPLE,
        llvm: llvm::LLVM_VERSION,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record<'a>(commit: &'a str, install: &'a str) -> BuildRecord<'a> {
        BuildRecord {
            version: "0.2.0",
            commit,
            built: "2026-10-26T00:00:00Z",
            install,
            binary: "/home/dev/.cargo/bin/rnx",
            stdlib: "registry cache",
            target: "x86_64-unknown-linux-gnu",
            llvm: "llvm22",
        }
    }

    #[test]
    fn install_kind_standalone_inputs() {
        assert_eq!(install_kind_label(None), "standalone");
        assert_eq!(install_kind_label(Some("")), "standalone");
    }

    #[test]
    fn install_kind_managed_input() {
        assert_eq!(
            install_kind_label(Some("managed by apt: apt upgrade rnx")),
            "managed:managed by apt: apt upgrade rnx"
        );
    }

    #[test]
    fn commit_falls_back_to_unknown() {
        assert_eq!(display_commit(""), "unknown");
        assert_eq!(display_commit("94399f4"), "94399f4");
    }

    #[test]
    fn built_falls_back_to_unknown() {
        assert_eq!(display_built(""), "unknown");
        assert_eq!(
            display_built("2026-10-26T00:00:00Z"),
            "2026-10-26T00:00:00Z"
        );
    }

    #[test]
    fn stdlib_labels_name_the_source() {
        assert_eq!(stdlib_label(None, None), "registry cache");
        assert_eq!(
            stdlib_label(Some("/src/stdlib"), None),
            "/src/stdlib (RNX_VENDOR_DIR)"
        );
        assert_eq!(stdlib_label(Some("  "), None), "registry cache");
        assert_eq!(
            stdlib_label(None, Some(Path::new("/repo/compiler/stdlib/src"))),
            "/repo/compiler/stdlib/src (source checkout; @std/* auto-vendored)"
        );
        assert_eq!(
            stdlib_label(
                Some("/vendor"),
                Some(Path::new("/repo/compiler/stdlib/src"))
            ),
            "/vendor (RNX_VENDOR_DIR)"
        );
    }

    #[test]
    fn long_shape_standalone() {
        let text = render_long(&record("94399f4", "standalone"));
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 8);
        assert_eq!(lines[0], "0.2.0");
        assert_eq!(lines[1], "commit: 94399f4");
        assert_eq!(lines[2], "built: 2026-10-26T00:00:00Z");
        assert_eq!(lines[3], "install: standalone");
        assert_eq!(lines[4], "binary: /home/dev/.cargo/bin/rnx");
        assert_eq!(lines[5], "stdlib: registry cache");
        assert_eq!(lines[6], "target: x86_64-unknown-linux-gnu");
        assert_eq!(lines[7], "llvm: llvm22");
        assert_eq!(format!("rnx {text}").lines().next().unwrap(), "rnx 0.2.0");
    }

    #[test]
    fn long_shape_managed_and_unknown_commit() {
        let text = render_long(&record("", "managed:managed by apt: apt upgrade rnx"));
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 8);
        assert_eq!(lines[0], "0.2.0");
        assert_eq!(lines[1], "commit: unknown");
        assert_eq!(lines[3], "install: managed:managed by apt: apt upgrade rnx");
    }

    #[test]
    fn long_shape_unknown_built_from_opt_out() {
        let mut rec = record("94399f4", "standalone");
        rec.built = "";
        let text = render_long(&rec);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 8);
        assert_eq!(lines[2], "built: unknown");
    }

    #[test]
    fn install_kind_empty_reason_is_standalone() {
        assert_eq!(install_kind_label(Some("")), "standalone");
        let text = render_long(&record("94399f4", &install_kind_label(None)));
        assert!(text.lines().any(|l| l == "install: standalone"), "{text}");
    }

    #[test]
    fn baked_long_starts_with_short_line() {
        let text = long_version();
        assert!(text.starts_with(&format!("{SHORT}\n")), "{text}");
        assert!(
            format!("rnx {text}").starts_with(&format!("{}\n", short_line())),
            "{text}"
        );
    }
}
