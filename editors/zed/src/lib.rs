use zed_extension_api as zed;

struct RasmalaiExtension;

impl zed::Extension for RasmalaiExtension {
    fn new() -> Self {
        RasmalaiExtension
    }

    fn language_server_command(
        &mut self,
        _language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<zed::Command> {
        // worktree.which() depends on Zed's shell-environment capture, which
        // can come up empty (e.g. unsupported login shells). Fall back to the
        // standard install locations before giving up.
        let command = worktree
            .which("rnx")
            .or_else(|| std::env::var("HOME").map(|h| format!("{h}/.cargo/bin/rnx")).ok())
            .unwrap_or_else(|| "/usr/local/bin/rnx".to_string());
        Ok(zed::Command {
            command,
            args: vec!["lsp".to_string()],
            env: Default::default(),
        })
    }
}

zed::register_extension!(RasmalaiExtension);
