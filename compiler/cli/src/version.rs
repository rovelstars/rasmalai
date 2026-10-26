pub const SHORT: &str = env!("CARGO_PKG_VERSION");
pub const COMMIT: &str = env!("RNX_COMMIT");
pub const BUILD_TIMESTAMP: &str = env!("RNX_BUILD_TIMESTAMP");
pub const INSTALL_REASON: &str = env!("RNX_DISTRO_MANAGED_REASON");
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

pub struct BuildRecord<'a> {
    pub version: &'a str,
    pub commit: &'a str,
    pub built: &'a str,
    pub install: &'a str,
    pub target: &'a str,
    pub llvm: &'a str,
}

pub fn render_long(r: &BuildRecord<'_>) -> String {
    format!(
        "{}\ncommit: {}\nbuilt: {}\ninstall: {}\ntarget: {}\nllvm: {}",
        r.version,
        display_commit(r.commit),
        display_built(r.built),
        r.install,
        r.target,
        r.llvm
    )
}

pub fn long_version() -> String {
    render_long(&BuildRecord {
        version: SHORT,
        commit: COMMIT,
        built: BUILD_TIMESTAMP,
        install: &install_kind_label(Some(INSTALL_REASON)),
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
        assert_eq!(display_built("2026-10-26T00:00:00Z"), "2026-10-26T00:00:00Z");
    }

    #[test]
    fn long_shape_standalone() {
        let text = render_long(&record("94399f4", "standalone"));
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 6);
        assert_eq!(lines[0], "0.2.0");
        assert_eq!(lines[1], "commit: 94399f4");
        assert_eq!(lines[2], "built: 2026-10-26T00:00:00Z");
        assert_eq!(lines[3], "install: standalone");
        assert_eq!(lines[4], "target: x86_64-unknown-linux-gnu");
        assert_eq!(lines[5], "llvm: llvm22");
        assert_eq!(format!("rnx {text}").lines().next().unwrap(), "rnx 0.2.0");
    }

    #[test]
    fn long_shape_managed_and_unknown_commit() {
        let text = render_long(&record("", "managed:managed by apt: apt upgrade rnx"));
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 6);
        assert_eq!(lines[0], "0.2.0");
        assert_eq!(lines[1], "commit: unknown");
        assert_eq!(lines[3], "install: managed:managed by apt: apt upgrade rnx");
    }

    #[test]
    fn baked_long_starts_with_short_line() {
        let text = long_version();
        assert!(text.starts_with(&format!("{SHORT}\n")), "{text}");
        assert!(format!("rnx {text}").starts_with(&format!("{}\n", short_line())), "{text}");
    }
}
