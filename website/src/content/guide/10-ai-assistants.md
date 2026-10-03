---
title: "AI Assistants & MCP"
description: "Connect Claude, Codex, Cursor, Pi, omp, opencode, Hermes, Cline, and Goose to Rasmalai through rnx mcp."
---

# AI Assistants & MCP

`rnx mcp` serves the toolchain over the Model Context Protocol on stdio, so an AI agent can check, run, format, and explain Rasmalai code without shelling out. Ten tools: `check`, `run` (interpreter, 30s cap), `fmt`, `explain`, `version`, `rasmalai_lookup_symbol` (`@std/*` signature search), `inspect_package_capabilities` (package security audit), `eval_code` (persistent JIT session), `get_diagnostics` (structured JSON diagnostics), and `hot_reload` (needs an attached `rnx dev --mcp` watcher). Stdout carries protocol frames only; everything works on self-contained snippets.

Pick your harness below. Every entry follows the same trio — `rnx` on `PATH`, a server entry with `command`/`args`, and one command to verify — then a manual section covers anything else.

Agents typically start by typechecking a snippet like this one through the `check` tool before touching your files:

```rnx
let total = [10, 20, 30].reduce(0, (acc, n) => acc + n);
assert(total == 60, "sum");
```

## <svg class="harness-logo" width="20" height="20" viewBox="0 0 24 24" fill="#c15f3c" role="img" aria-label="Claude"><path d="m4.7144 15.9555 4.7174-2.6471.079-.2307-.079-.1275h-.2307l-.7893-.0486-2.6956-.0729-2.3375-.0971-2.2646-.1214-.5707-.1215-.5343-.7042.0546-.3522.4797-.3218.686.0608 1.5179.1032 2.2767.1578 1.6514.0972 2.4468.255h.3886l.0546-.1579-.1336-.0971-.1032-.0972L6.973 9.8356l-2.55-1.6879-1.3356-.9714-.7225-.4918-.3643-.4614-.1578-1.0078.6557-.7225.8803.0607.2246.0607.8925.686 1.9064 1.4754 2.4893 1.8336.3643.3035.1457-.1032.0182-.0728-.164-.2733-1.3539-2.4467-1.445-2.4893-.6435-1.032-.17-.6194c-.0607-.255-.1032-.4674-.1032-.7285L6.287.1335 6.6997 0l.9957.1336.419.3642.6192 1.4147 1.0018 2.2282 1.5543 3.0296.4553.8985.2429.8318.091.255h.1579v-.1457l.1275-1.706.2368-2.0947.2307-2.6957.0789-.7589.3764-.9107.7468-.4918.5828.2793.4797.686-.0668.4433-.2853 1.8517-.5586 2.9021-.3643 1.9429h.2125l.2429-.2429.9835-1.3053 1.6514-2.0643.7286-.8196.85-.9046.5464-.4311h1.0321l.759 1.1293-.34 1.1657-1.0625 1.3478-.8804 1.1414-1.2628 1.7-.7893 1.36.0729.1093.1882-.0183 2.8535-.607 1.5421-.2794 1.8396-.3157.8318.3886.091.3946-.3278.8075-1.967.4857-2.3072.4614-3.4364.8136-.0425.0304.0486.0607 1.5482.1457.6618.0364h1.621l3.0175.2247.7892.522.4736.6376-.079.4857-1.2142.6193-1.6393-.3886-3.825-.9107-1.3113-.3279h-.1822v.1093l1.0929 1.0686 2.0035 1.8092 2.5075 2.3314.1275.5768-.3218.4554-.34-.0486-2.2039-1.6575-.85-.7468-1.9246-1.621h-.1275v.17l.4432.6496 2.3436 3.5214.1214 1.0807-.17.3521-.6071.2125-.6679-.1214-1.3721-1.9246L14.38 17.959l-1.1414-1.9428-.1397.079-.674 7.2552-.3156.3703-.7286.2793-.6071-.4614-.3218-.7468.3218-1.4753.3886-1.9246.3157-1.53.2853-1.9004.17-.6314-.0121-.0425-.1397.0182-1.4328 1.9672-2.1796 2.9446-1.7243 1.8456-.4128.164-.7164-.3704.0667-.6618.4008-.5889 2.386-3.0357 1.4389-1.882.929-1.0868-.0062-.1579h-.0546l-6.3385 4.1164-1.1293.1457-.4857-.4554.0608-.7467.2307-.2429 1.9064-1.3114Z"/></svg> Claude Code & Desktop

Claude Code registers from the terminal:

```sh
claude mcp add rnx -- rnx mcp
```

Verify with `/mcp` inside a session; the `rnx` server lists ten tools.

Claude Desktop takes the same server in its JSON config — macOS `~/Library/Application Support/Claude/claude_desktop_config.json` (Windows `%APPDATA%\Claude\`, Linux `~/.config/Claude`):

```json
{
  "mcpServers": {
    "rnx": { "command": "rnx", "args": ["mcp"] }
  }
}
```

Restart Desktop after editing.

## <img src="https://cdn.jsdelivr.net/npm/simple-icons@latest/icons/openai.svg" alt="Codex" width="20" height="20" class="harness-logo harness-inv" /> Codex

Codex CLI reads `~/.codex/config.toml`:

```toml
[mcp_servers.rnx]
command = "rnx"
args = ["mcp"]
```

## <img src="https://cdn.jsdelivr.net/npm/simple-icons@latest/icons/cursor.svg" alt="Cursor" width="20" height="20" class="harness-logo harness-inv" /> Cursor

Project file `.cursor/mcp.json` (global: Cursor Settings -> MCP -> Add):

```json
{
  "mcpServers": {
    "rnx": { "command": "rnx", "args": ["mcp"] }
  }
}
```

## <img src="https://cdn.jsdelivr.net/npm/simple-icons@latest/icons/pi.svg" alt="Pi" width="20" height="20" class="harness-logo harness-inv" /> Pi

Global `~/.pi/agent/mcp.json`, or `.mcp.json` in a trusted project:

```json
{
  "mcpServers": {
    "rnx": { "command": "rnx", "args": ["mcp"] }
  }
}
```

Reload with `/mcp reload`.

## <img src="https://raw.githubusercontent.com/can1357/oh-my-pi/HEAD/assets/icon.svg" alt="omp" width="26" height="20" class="harness-omp" /> omp

Project `.omp/mcp.json`, or user `~/.omp/mcp.json` (`/mcp add` walks you through it, `/mcp test rnx` checks the connection):

```json
{
  "mcpServers": {
    "rnx": { "type": "stdio", "command": "rnx", "args": ["mcp"] }
  }
}
```

## <img src="https://cdn.jsdelivr.net/npm/simple-icons@latest/icons/opencode.svg" alt="opencode" width="20" height="20" class="harness-logo harness-inv" /> opencode

`opencode.json` in the project (or `~/.config/opencode/opencode.json` globally):

```json
{
  "mcp": {
    "rnx": { "type": "local", "command": ["rnx", "mcp"] }
  }
}
```

## <img src="https://hermes-agent.nousresearch.com/favicon.ico" alt="Hermes" width="20" height="20" class="harness-logo" />Hermes agent

`~/.hermes/config.yaml` under the top-level `mcp_servers` key (that key name is load-bearing — `mcp: servers:` will not load):

```yaml
mcp_servers:
  rnx:
    command: "rnx"
    args: ["mcp"]
```

Test with `hermes mcp test rnx`, then `/reload-mcp` in a live session.

## <img src="https://cdn.jsdelivr.net/npm/simple-icons@latest/icons/cline.svg" alt="Cline" width="20" height="20" class="harness-logo harness-inv" /> Cline

Open **Cline: Open MCP Settings** from the VS Code command palette (`cline_mcp_settings.json`), then add:

```json
{
  "mcpServers": {
    "rnx": { "command": "rnx", "args": ["mcp"] }
  }
}
```

## <img src="https://goose-docs.ai/img/logo_dark.png" alt="Goose" width="20" height="20" class="harness-logo harness-only-dark" /><img src="https://goose-docs.ai/img/logo_light.png" alt="Goose" width="20" height="20" class="harness-logo harness-only-light" />Goose

`~/.config/goose/config.yaml` (or `goose configure` -> Extensions -> Add extension):

```yaml
extensions:
  - name: "rnx"
    enabled: true
    transport:
      type: "stdio"
      command: "rnx"
      args: ["mcp"]
```

## Manual setup for the rest

Any harness that spawns a stdio MCP server needs exactly three things: the `rnx` binary on `PATH`, `command` set to `rnx` with `args` set to `["mcp"]`, and a clean stdout (the server reserves it for protocol frames; logs go to stderr). Verify by hand before blaming the harness:

```sh
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}' '{"jsonrpc":"2.0","method":"notifications/initialized"}' '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' | rnx mcp
```

Expect `serverInfo.name` of `rnx-mcp` and ten tools. If a harness rejects the server, check that it passes through `env`/`cwd` untouched and that no wrapper writes banners to stdout.

Treat `rnx mcp` as local-trust software: `run` executes the code it is given with your user privileges, capped at 30 seconds per call. The full tool table and protocol notes live in the [CLI reference](/manual/16-project-and-toolchain).
