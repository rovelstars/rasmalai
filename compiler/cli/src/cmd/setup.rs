use std::path::PathBuf;

const SUPPORTED: &[&str] = &["vscode", "zed", "helix", "neovim"];

const HELIX_STANZA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../editors/helix/languages.toml"
));

const NEOVIM_PLUGIN: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../editors/neovim/rasmalai.lua"
));

const ZED_SETTINGS_SNIPPET: &str = r#"{
  "lsp": { "rnx-lsp": { "binary": { "path": "rnx", "args": ["lsp"] } } },
  "languages": { "Rasmalai": { "language_servers": ["rnx-lsp"] } }
}"#;

pub(super) fn run_setup(editor: Option<String>) {
    let name = match editor.as_deref() {
        Some(e) => e.to_lowercase(),
        None => {
            eprintln!("usage: rnx setup <editor>");
            eprintln!("supported editors: {}", SUPPORTED.join(", "));
            std::process::exit(2);
        }
    };
    match name.as_str() {
        "vscode" => setup_vscode(),
        "zed" => setup_zed(),
        "helix" => setup_helix(),
        "neovim" => setup_neovim(),
        other => {
            eprintln!("unknown editor `{other}`");
            eprintln!("supported editors: {}", SUPPORTED.join(", "));
            std::process::exit(2);
        }
    }
}

fn fail(message: String) -> ! {
    eprintln!("error: {message}");
    std::process::exit(1);
}

fn home_dir() -> PathBuf {
    match std::env::var_os("HOME") {
        Some(h) if !h.is_empty() => PathBuf::from(h),
        _ => fail("cannot determine home directory (HOME is not set)".to_string()),
    }
}

fn config_dir() -> PathBuf {
    match std::env::var_os("XDG_CONFIG_HOME") {
        Some(x) if !x.is_empty() => PathBuf::from(x),
        _ => home_dir().join(".config"),
    }
}

fn ancestors() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            roots.push(dir.to_path_buf());
        }
    }
    roots
}

fn find_in_checkout(rel: &str) -> Option<PathBuf> {
    for base in ancestors() {
        let mut dir = Some(base.as_path());
        while let Some(d) = dir {
            let cand = d.join("editors").join(rel);
            if cand.exists() {
                return Some(cand);
            }
            dir = d.parent();
        }
    }
    None
}

fn setup_helix() {
    let target = config_dir().join("helix").join("languages.toml");
    if target.is_file() {
        let existing = std::fs::read_to_string(&target)
            .unwrap_or_else(|e| fail(format!("cannot read {}: {e}", target.display())));
        if existing.contains("name = \"rasmalai\"") {
            println!("helix: already configured in {}", target.display());
            println!("Next: restart Helix and open a .rnx file; `rnx lsp` starts automatically.");
            return;
        }
        let backup = target.with_extension("toml.bak");
        std::fs::copy(&target, &backup)
            .unwrap_or_else(|e| fail(format!("cannot back up {}: {e}", target.display())));
        let mut merged = existing;
        if !merged.ends_with('\n') {
            merged.push('\n');
        }
        merged.push('\n');
        merged.push_str(HELIX_STANZA);
        std::fs::write(&target, merged)
            .unwrap_or_else(|e| fail(format!("cannot write {}: {e}", target.display())));
        println!("helix: appended rasmalai stanza to {}", target.display());
        println!("backup: {}", backup.display());
        println!("Next: restart Helix and open a .rnx file; `rnx lsp` starts automatically.");
        return;
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|e| fail(format!("cannot create {}: {e}", parent.display())));
    }
    std::fs::write(&target, HELIX_STANZA)
        .unwrap_or_else(|e| fail(format!("cannot write {}: {e}", target.display())));
    println!("helix: wrote {}", target.display());
    println!("Next: restart Helix and open a .rnx file; `rnx lsp` starts automatically.");
}

fn setup_neovim() {
    let target = config_dir()
        .join("nvim")
        .join("lua")
        .join("rasmalai.lua");
    if target.is_file() {
        let existing = std::fs::read_to_string(&target)
            .unwrap_or_else(|e| fail(format!("cannot read {}: {e}", target.display())));
        if existing == NEOVIM_PLUGIN {
            println!("neovim: already configured in {}", target.display());
            println!("Next: ensure `require(\"rasmalai\")` runs from your init.lua, restart Neovim, and open a .rnx file.");
            return;
        }
        let backup = target.with_extension("lua.bak");
        std::fs::copy(&target, &backup)
            .unwrap_or_else(|e| fail(format!("cannot back up {}: {e}", target.display())));
        std::fs::write(&target, NEOVIM_PLUGIN)
            .unwrap_or_else(|e| fail(format!("cannot write {}: {e}", target.display())));
        println!("neovim: updated {}", target.display());
        println!("backup: {}", backup.display());
        println!("Next: ensure `require(\"rasmalai\")` runs from your init.lua, restart Neovim, and open a .rnx file.");
        return;
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|e| fail(format!("cannot create {}: {e}", parent.display())));
    }
    std::fs::write(&target, NEOVIM_PLUGIN)
        .unwrap_or_else(|e| fail(format!("cannot write {}: {e}", target.display())));
    println!("neovim: wrote {}", target.display());
    println!("Next: add `require(\"rasmalai\")` to your init.lua, restart Neovim, and open a .rnx file.");
}

fn code_present() -> bool {
    std::process::Command::new("code")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn find_packaged_vsix() -> Option<PathBuf> {
    let dir = find_in_checkout("vscode")?;
    let mut vsix: Vec<PathBuf> = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "vsix"))
        .collect();
    vsix.sort();
    vsix.into_iter().next()
}

fn setup_vscode() {
    if code_present() {
        if let Some(vsix) = find_packaged_vsix() {
            match std::process::Command::new("code")
                .arg("--install-extension")
                .arg(&vsix)
                .output()
            {
                Ok(out) if out.status.success() => {
                    println!("vscode: installed {}", vsix.display());
                    println!("Next: reload VS Code, open a .rnx file, and confirm `rnx` is on PATH for the LSP.");
                    return;
                }
                Ok(out) => fail(format!(
                    "code --install-extension failed: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                )),
                Err(e) => fail(format!("cannot run `code`: {e}")),
            }
        }
    }
    let source = find_in_checkout("vscode")
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "<checkout>/editors/vscode".to_string());
    println!("vscode: no packaged .vsix found; manual install:");
    println!("  1. package the extension: cd {source} && npx vsce package");
    println!("  2. install it: code --install-extension rasmalai-*.vsix");
    println!("  3. ensure `rnx` is on PATH, then reload VS Code and open a .rnx file.");
    println!("note: the extension is not on the Marketplace; install from the local source above.");
}

fn setup_zed() {
    let path = find_in_checkout("zed/extension.toml")
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "<checkout>/editors/zed".to_string());
    println!("zed: dev-extension install (no registry package exists):");
    println!("  1. Zed > Extensions > Install Dev Extension > select {path}");
    println!("  2. add to ~/.config/zed/settings.json:");
    println!("{ZED_SETTINGS_SNIPPET}");
    println!("  3. ensure `rnx` is on PATH, then restart Zed and open a .rnx file.");
}
