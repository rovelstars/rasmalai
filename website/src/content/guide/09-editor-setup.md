---
title: "Editor Setup"
description: "Syntax highlighting and rnx lsp in VS Code, Zed, Helix, and Neovim, through rnx setup or by hand."
icon: "Monitor"
---

# Editor Setup

Every editor integration talks to the same server: `rnx lsp` over stdio. It typechecks the open files on every change and pushes back errors and lint warnings with their `E`/`L` codes. The fastest way to get it running is the setup command:

```sh
rnx setup vscode   # or: zed, helix, neovim
```

`rnx` itself must be on `PATH` first — that is the one thing every editor needs. If it is not installed yet:

```sh
curl -fsSL https://rnx.dev/install.sh | sh
rnx --version
```

Each `rnx setup <editor>` prints exactly what it changed and what to do next. Unknown names fail with the supported list, so `rnx setup emacs` tells you the four it knows. The rest of this page shows what the command does per editor, so you can also do it by hand.

Check that the server answers before blaming the editor:

```rnx
fn Main(): Int {
    let total = [10, 20, 30].reduce(0, (acc, n) => acc + n);
    assert(total == 60, "sum");
    return total - 60;
}
```

```sh
rnx check main.rnx
```

Exit code `0` with no output means the file is clean — the same result the editor should show as zero squiggles.

## <img src="https://cdn.jsdelivr.net/npm/simple-icons@latest/icons/visualstudiocode.svg" alt="VS Code" width="20" height="20" class="harness-logo harness-inv" /> VS Code

```sh
rnx setup vscode
```

The command installs the packaged extension when it finds a local `.vsix` next to the checkout and the `code` CLI on `PATH`. Otherwise it prints manual steps, because the extension is not on the Marketplace — install it from source:

```sh
cd editors/vscode
npx vsce package
code --install-extension rasmalai-*.vsix
```

The extension activates on `*.rnx` files and starts `rnx lsp` with `rnx` from `PATH`. If your binary lives elsewhere, point it at the right one in settings:

```json
{
  "rasmalai.serverPath": "/home/you/.rnx/bin/rnx"
}
```

Reload the window after installing, then open a `.rnx` file. Hover, go-to-definition, and diagnostics all come from the running server.

## <img src="https://zed.dev/favicon_black_64.png" alt="Zed" width="20" height="20" class="harness-logo harness-only-light" /><img src="https://zed.dev/favicon_white_64.png" alt="Zed" width="20" height="20" class="harness-logo harness-only-dark" /> Zed

```sh
rnx setup zed
```

Zed has no registry package for Rasmalai, so setup is a dev extension. The command prints these steps with the exact checkout path:

1. Open Zed, then Extensions → Install Dev Extension, and select `editors/zed` in your Rasmalai checkout.
2. Add the language server to `~/.config/zed/settings.json`:

```json
{
  "lsp": { "rnx-lsp": { "binary": { "path": "rnx", "args": ["lsp"] } } },
  "languages": { "Rasmalai": { "language_servers": ["rnx-lsp"] } }
}
```

3. Confirm `rnx` is on `PATH`, restart Zed, and open a `.rnx` file.

## File icons

The canonical `.rnx` artwork is the `rnx` glyph kept next to the
website logo (`website/static/favicon.svg`, copied for packaging into
`editors/vscode/icons/rasmalai.svg`). Icon packs that do not know
Rasmalai fall back as follows:

- **VS Code**: the Rasmalai extension ships a minimal `Rasmalai Icons`
  file-icon theme covering `.rnx` only. Pick it in
  File → Preferences → File Icon Theme when your main theme shows a
  generic icon. It is a fallback, not a full theme: it styles `.rnx`
  files and leaves everything else to VS Code defaults.
- **Zed**: extensions cannot ship custom file icons — Zed renders its
  built-in generic icon and there is nothing to configure. Upstream
  icon support is the only fix; the canonical SVG above is what to
  point it at.
- **Helix / Neovim**: terminal UIs have no file icons to theme.

## <img src="https://cdn.jsdelivr.net/npm/simple-icons@latest/icons/helix.svg" alt="Helix" width="20" height="20" class="harness-logo harness-inv" /> Helix

```sh
rnx setup helix
```

The command appends `editors/helix/languages.toml` to `~/.config/helix/languages.toml` (or `$XDG_CONFIG_HOME/helix/languages.toml`), backing up your existing file to `languages.toml.bak` first. Running it twice is safe — it detects the stanza and stops. By hand, merge this into yours:

```toml
[[language]]
name = "rasmalai"
scope = "source.rnx"
file-types = ["rnx", "Project.config", "Project.deplock"]
roots = ["Project.config", ".git"]
comment-tokens = ["//"]
block-comment-tokens = [{ start = "/*", end = "*/" }]
indent = { tab-width = 4, unit = "    " }
language-servers = ["rnx-lsp"]

[language-server.rnx-lsp]
command = "rnx"
args = ["lsp"]

# Rasmalai manifests are `.rnx` modules; highlight them as such.
```

Restart Helix and open a `.rnx` file; the server starts on its own. `Project.config` files get Rasmalai highlighting too, since they are `.rnx` modules.

## <img src="https://cdn.jsdelivr.net/npm/simple-icons@latest/icons/neovim.svg" alt="Neovim" width="20" height="20" class="harness-logo harness-inv" /> Neovim

```sh
rnx setup neovim
```

The command copies `editors/neovim/rasmalai.lua` to `~/.config/nvim/lua/rasmalai.lua` (existing files are backed up to `rasmalai.lua.bak`), then asks you to source it from your `init.lua`:

```lua
require("rasmalai")
```

The plugin itself sets the `rasmalai` filetype and starts the server per buffer:

```lua
vim.filetype.add({
    extension = {
        rnx = "rasmalai",
    },
    filename = {
        ["Project.config"] = "rasmalai",
        ["Project.deplock"] = "rasmalai",
    },
})

vim.api.nvim_create_autocmd("FileType", {
    pattern = "rasmalai",
    callback = function()
        vim.lsp.start({
            name = "rasmalai",
            cmd = { "rnx", "lsp" },
            root_dir = vim.fs.root(0, { "Project.config", ".git" }),
        })
    end,
})
```

Restart Neovim and open a `.rnx` file. The server roots itself at `Project.config` (or `.git`) so diagnostics resolve imports across the project.

## When it does not work

- `rnx: command not found` in the editor log means `PATH` differs between your shell and the editor. Launch the editor from the shell once, or set an absolute binary path (VS Code's `rasmalai.serverPath`, Zed's `binary.path`).
- Stale squiggles after an upgrade mean the editor kept an old server alive. Restart the editor so it respawns `rnx lsp` from the new binary.
- To see the raw protocol, most clients have a trace setting — VS Code exposes it as `rasmalai.trace.server`. Server-side log lines go to stderr, never to the protocol stream.

From here, the [AI Assistants](/guide/10-ai-assistants) page wires the same toolchain into agents through `rnx mcp`.
