---
title: "Troubleshooting & Reinstall"
description: "Read a diagnostic, fix the common breakages, and reinstall cleanly on Linux, macOS, and Windows."
---

# Troubleshooting & Reinstall

Most Rasmalai problems fall into three buckets: a diagnostic you can read, a stale project, or a stale toolchain. Work them in that order — nine times out of ten you never reach step three.

## Read the diagnostic first

Every error carries a code. `rnx check` shows it with a span pointing at the exact column:

```sh
rnx check
```

Then ask the toolchain what the code means:

```sh
rnx explain E108
```

The explanation names the cause and the fix. The full directory of codes lives in the [Language Manual](/manual/17-diagnostics-directory). If a message looks wrong — a span pointing nowhere, a code with no entry — that is a compiler bug: skip to [filing an issue](#filing-an-issue) and paste the output verbatim.

When output looks suspiciously terse, rerun with the pipeline log turned on:

```sh
rnx -v check
```

Verbose mode prints each pass as it runs, which tells you whether the failure is in parsing, typechecking, or code generation.

A healthy program to compare against:

```rnx
let total = [10, 20, 30].reduce(0, (acc, n) => acc + n);
assert(total == 60, "sum");
```

If the snippet above checks clean but your project does not, the problem is in your project, not the toolchain. Keep reading.

## Fix the project before the toolchain

Three commands resolve most project-level breakage:

```sh
rnx fmt
rnx check
rnx test
```

`rnx fmt` normalizes layout so you stop chasing whitespace ghosts. `rnx check` revalidates in milliseconds. `rnx test` reruns the suite so you know the fix did not move the failure elsewhere.

Dependency trouble has its own shape: a `fetch` that worked yesterday fails today, or a type from a git dependency suddenly mismatches. Dependencies resolve into `Project.deplock`; when the lockfile disagrees with the manifests, re-resolve from scratch:

```sh
rm Project.deplock
rnx fetch
rnx check
```

Editor acting up — stale squiggles, completions from another era — is almost always the language server holding onto a closed project. Restart it through the setup command rather than reinstalling anything:

```sh
rnx setup
```

The [editor setup chapter](/guide/09-editor-setup) covers per-editor details.

## Reinstall the toolchain cleanly

Reach for this when `rnx --version` itself fails, when the binary predates the error you are seeing, or when support asks you to rule out a corrupted install. The procedure is the same on every OS: remove the old install, then install fresh. Uninstallation is just step one of a reinstall — there is no separate ritual.

First, confirm what you have:

```sh
rnx --version
which rnx
```

### Linux

Remove the old toolchain (default locations; adjust if you passed `--prefix` at install time):

```sh
rm -rf ~/.local/bin/rnx ~/.local/bin/rnx.bin ~/.local/bin/rnx.exe ~/.local/lib
```

Then reinstall:

```sh
curl -fsSL https://rnx.dev/install.sh | sh
```

The installer honors `$XDG_BIN_HOME` when it is set, otherwise it uses `~/.local` — the default user prefix from the XDG base directory spec. To put it elsewhere:

```sh
curl -fsSL https://rnx.dev/install.sh | sh -s -- --prefix /usr/local
```

### macOS

Same layout as Linux — `~/.local` is the conventional home for CLI tools outside the App Store, and `$XDG_BIN_HOME` is honored the same way. Rasmalai ships Apple Silicon builds only; Intel Macs run the toolchain under Rosetta or not at all.

```sh
rm -rf ~/.local/bin/rnx ~/.local/bin/rnx.bin ~/.local/lib
curl -fsSL https://rnx.dev/install.sh | sh
```

### Windows

If you installed with the PowerShell installer (default `%LOCALAPPDATA%\rnx`), remove that directory and drop its `bin` from the user `PATH`:

```powershell
Remove-Item $env:LOCALAPPDATA\rnx -Recurse -Force
```

Then open Settings → System → About → Advanced system settings → Environment Variables, edit the user `Path`, and delete the `...\rnx\bin` entry. Then reinstall in a fresh terminal:

```powershell
irm https://rnx.dev/install.ps1 | iex
```

If you installed under Git Bash or MSYS2 instead, remove the Unix-side prefix and rerun the shell installer:

```sh
rm -rf ~/.local/bin/rnx* ~/.local/lib
curl -fsSL https://rnx.dev/install.sh | sh
```

After any reinstall, verify in a new terminal (new, so `PATH` changes take effect):

```sh
rnx --version
```

## Filing an issue

If the steps above did not fix it, file it — a clean repro against the latest toolchain is exactly what maintainers need. Include all four:

1. `rnx --version` output.
2. Your OS and CPU (`uname -sm`, or `$PSVersionTable` on Windows).
3. The smallest source file that still fails.
4. The full command and its complete output (with `rnx -v` if it is a compiler crash).

Open the issue at [github.com/rovelstars/rasmalai](https://github.com/rovelstars/rasmalai). Reproducible reports get fixed first; "it broke" with no version gets asked for a version.
