use std::borrow::Cow;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, GetPromptRequestParams, GetPromptResponse, GetPromptResult,
    Implementation, ListPromptsResult, ListResourcesResult, PaginatedRequestParams, Prompt,
    PromptArgument, PromptMessage, ProtocolVersion, ReadResourceRequestParams, ReadResourceResponse,
    ReadResourceResult, Resource, ResourceContents, Role, ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::{tool, tool_handler, tool_router, ErrorData as McpError, RoleServer};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::dev::{DevCommand, HotReport};
use crate::repl::{ReplOut, ReplSession};
use crate::RunOutcome;

const RUN_TIMEOUT: Duration = Duration::from_secs(30);
const JOB_TIMEOUT: Duration = Duration::from_secs(180);
const EVAL_CODE_CAP: usize = 32 * 1024;
const OUTPUT_CAP: u64 = 32 * 1024;
const OUTPUT_MAX: u64 = 256 * 1024;
const DIAG_CAP: usize = 100;
const TEXT_CAP: usize = 8 * 1024;

fn run_timeout() -> Duration {
    std::env::var("RNX_MCP_RUN_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|s| *s > 0)
        .map(Duration::from_secs)
        .unwrap_or(RUN_TIMEOUT)
}

fn job_timeout() -> Duration {
    std::env::var("RNX_MCP_JOB_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|s| *s > 0)
        .map(Duration::from_secs)
        .unwrap_or(JOB_TIMEOUT)
}

const INSTRUCTIONS: &str = "You are an expert compiler and software engineer specializing in Rasmalai. \
Rasmalai is a high-performance systems language with the ergonomics of a modern web language.\n\n\
Core Invariants:\n\
1. Unified 64-bit Numerics: Only `Int` and `Float` exist. There are NO sized integers (no `u8`, `i32`, `u64`).\n\
2. Bindings: Variables declared with `let` are mutable by default. There is NO `mut` keyword.\n\
3. Constructors: Classes use `new`: `new Meter(20)` allocates the object and runs `init` (calling a class directly, `Meter(20)`, is E204). Structs and records use direct calls: `Point(1.0, 2.0)` (`new` on a struct is an error).\n\
4. Control Flow: Single-statement `if` and `else` bodies are valid, but lexical declarations (`let`) inside single-statement bodies are forbidden (E108). Lone semicolons `;` are valid empty statements.\n\
5. Memory and Cycles: Memory is managed via deterministic scope-based ARC with write-dominance cleanup on loop back-edges. Cyclical data structures MUST NOT use raw pointer loops; use GenRefs (`GenRef.of(obj)`, stale `.get()` returns null).\n\n\
Tool Protocol:\n\
6. Default to `rasmalai_lookup_symbol` for `@std/*` APIs. Never read the full `rasmalai://stdlib/api.json` catalog first (300 KB+); per-module `rasmalai://stdlib/<module>.json` resources exist for browsing one module.\n\
7. Validate with `get_diagnostics` (structured JSON, machine-first) or `check` (same verdicts, `pretty: true` for readable output). `path` inputs are project-aware: relative imports resolve against the file, so prefer `path` for multi-file projects. `source` snippets are self-contained.\n\
8. `eval_code` persists JIT session state; the first eval pays JIT warmup. Use `:type <expr>` for static types, `:reset` (or `reset: true`) to clear state.\n\
9. `run` caps output (`max_output_bytes`) and enforces a 30 s limit; a timed-out worker blocks further `run`/`build` calls until it exits. `build` and `test` cap output the same way; `build --release` runs LLVM and is slow.\n\
10. Publishing, dependency mutation, and dev-watcher spawning are intentionally unavailable over MCP; use the `rnx` CLI for those.\n\n\
Entry point is `fn Main(): Int`. Read `rasmalai://spec/packages` before touching manifests, locks, or the cache.";

const GRAMMAR_SPEC: &str = "# Rasmalai Grammar\n\n\
## Numerics\n\n\
Only two numeric types exist: `Int` (64-bit signed) and `Float` (IEEE-754 double). \
There are no sized integers (`u8`, `i32`, `u64` do not exist). Integer literals are `Int`; \
a literal with a decimal point or exponent (`1.0`, `2e3`) is `Float`. Integer lanes read from \
byte buffers widen into 64-bit `Int`; float lanes widen into `Float`.\n\n\
## Strings\n\n\
Double-quoted strings support `${...}` interpolation: `\"hello ${name}\"`. \
Adjacent string literals do not concatenate implicitly.\n\n\
## Operators\n\n\
Arithmetic `+ - * / %`, comparison `== != < <= > >=`, logical `&& || !`, nullish `??`, \
optional access `?.` (must directly touch its operand: `a?.b`), ranges via `..`. \
User types overload operators with `op_add`, `op_sub`, `op_index`, `op_index_set`, and siblings.\n\n\
## Functions\n\n\
```\nfn greet(name: String = \"world\"): String {\n    return \"hi ${name}\";\n}\n```\n\n\
Parameters carry explicit types and may declare defaults. A trailing `throws` marks a \
throwing signature; unmarked functions are pure with respect to errors. The program entry \
is `fn Main(): Int`.\n\n\
## Types\n\n\
- `struct`: value type with methods (e.g. `struct Point { let x: Float; let y: Float; }`). \
Constructed by direct call: `Point(1.0, 2.0)` (`new` on a struct is an error).\n\
- `record`: positional, immutable value type: `record Point(x: Float, y: Float)`, also built by direct call.\n\
- `class`: heap reference type managed by ARC. Constructed with `new`: `new Meter(20)` allocates \
and runs `init`; calling a class directly (`Meter(20)`) is E204.\n\
- `trait`: shared behavior composed into classes with `with`: \
`class C extends Base with TraitA, TraitB { ... }`.\n\
- `interface`: dynamic dispatch surface.\n\
- Bindings use `let` and are mutable by default; no `mut` keyword exists.\n\n\
## Modules and imports\n\n\
Each file is its own namespace; `public` items import freely between project files in any direction. \
Relative imports probe `<path>.rnx`, `<path>/mod.rnx`, `<path>/index.rnx` in order:\n\n\
```\nimport { helper } from \"./util\";\nimport math from \"@std/math\";\n```\n\n\
`@std/*` modules need no manifest entry and no version pin. Bare and scoped specifiers name registry \
or path dependencies from the nearest `Project.config`.\n\n\
## Control flow\n\n\
Single-statement `if` and `else` bodies are valid, but a lexical declaration (`let`) inside a \
single-statement body is rejected (E108): wrap the body in braces. A lone `;` is an empty statement.";

const ARCHITECTURE_SPEC: &str = "# Rasmalai Architecture\n\n\
## Scope-based deterministic ARC\n\n\
Every heap value is reference-counted with retains and releases placed at lexical scope exits: \
containers, copies, loads, and call arguments retain; scope end, destructors, and returns release. \
There is no tracing garbage collector and therefore no GC pauses. Releasing the old value on \
reassignment only fires when the destination is provably owned and the write dominates the read.\n\n\
## Write-dominance on loop back-edges\n\n\
Loop back-edges gate temporary cleanup: a value created inside a loop body is released only on \
paths dominated by its defining write. This keeps loop-carried values alive across iterations \
while still freeing per-iteration temporaries deterministically.\n\n\
## Generational references (GenRefs)\n\n\
Rule: ARC owns, `GenRef` points back. Cyclical structures (parents, observers, scene-graph edges) \
must use `GenRef<T>` from the prelude instead of strong reference cycles: `let parent: GenRef<Window>` \
with `GenRef.of(parent)` to create one. A `GenRef` is a slot handle plus an expected generation; \
when the owner frees the slot its generation bumps, so a stale `.get()` returns null instead of \
dangling. Mutually strong fields (`A` contains `B`, `B` contains `A`) raise W108 and should convert \
one side to `GenRef` (or carry an explicit opt-out for manually broken cycles).\n\n\
## Relation to other systems languages\n\n\
- Rust: no lifetime annotations and no borrow-checker wrestling; safety comes from deterministic \
ARC over lexical scopes plus `GenRef` back-edges.\n\
- Go: no runtime-scheduled green threads and no GC pauses; concurrency comes from explicit \
threadpools, mutexes, and barriers in `@std/sync`.\n\
- C++: no undefined behavior in safe code and no manual `new`/`delete`; ARC is the default and \
raw hardware interop stays behind C-ABI FFI boundaries.";

const MANIFEST_SPEC: &str = "# Project.config Manifest\n\n\
Every project carries a `Project.config` manifest written as an `.rnx` module exporting a default object (sibling `Project.deplock` pins resolved deps).\n\n\
```rnx\nexport default {\n    project: {\n        name: \"my-service\",\n        version: \"0.1.0\",\n        engine: \">=0.4.0\"\n    },\n    entries: {\n        main: \"src/main.rnx\"\n    },\n    dependencies: {\n        locallib: \"libs/local-lib\",\n        httpkit: \"^3.45.0\",\n        uikit: \"~1.2.0\"\n    }\n}\n```\n\n\
## Sections\n\n\
- `project`: `name` and `version` are required; \
`engine` is the minimum toolchain requirement (e.g. `\">=0.4.0\"`).\n\
- `entries` (optional): `main` defaults to `src/main.rnx`; `lib` names the library file, `docs` the guides folder, `bins` extra tool shims.\n\
- `dependencies`: one entry per dependency; only one source kind per entry.\n\
- `registry` (optional): registry endpoints for published packages.\n\
- `permissions` (optional): capability strings or `{ perm, reason }` tables forming the security ceiling.\n\
- `workspace` (optional): `{ members: [...] }` monorepo member list.\n\n\
## Dependency sources\n\n\
- SemVer requirement as a bare string: `\"^1.2.0\"` (compatible), `\"~1.2.0\"` (patch-level), \
exact `\"1.5.5\"`, or ranges combined per SemVer 2.0 comparators.\n\
- Path: `\"libs/x\"` or `{ path: \"...\" }`.\n\
- Git: `{ git: \"<url>\", rev/tag/branch = \"...\" }` (exactly one of `rev`, `tag`, `branch`).\n\
- Tarball: `{ version: \"...\", url = \"...\", checksum = \"...\" }`.\n\
- Native: `{ native: \"z\", system: true }` for system C libraries.\n\n\
## Config logic\n\n\
Manifests are evaluated by a sandboxed const evaluator: top-level `const`, object/array spreads (`...deps`), \
ternaries, `switch` on the ambient `target` object (`target.os`, `target.arch`, `target.env`), and member access. \
Functions, loops, imports, and I/O are rejected.\n\n\
## Standard modules\n\n\
`@std/*` modules are built in and need no manifest entry: \
`import { ByteBuffer } from \"@std/bytes\";` resolves without a `dependencies` line. \
Use `rnx lock` to write `Project.deplock` and `rnx vendor` to copy sources into `.vendor/`.";

const PACKAGES_SPEC: &str = "# Package management lifecycle\n\n\
## The loop\n\n\
`rnx add <name>[@range]` records a dependency (`^<version>` by default) after capability approval. \
`rnx lock` resolves the transitive graph, checksums every package directory, and writes a sorted \
deterministic `Project.deplock` carrying per-package tier, capabilities, license, and link choice. \
`rnx fetch` downloads scope dependencies and precompiles their products; `rnx download` seeds the \
registry cache (`@std/*` when named without args); `rnx vendor` stages local sources. \
`rnx doctor` reports cache health; `rnx cache status` reports pool sizes. Builds verify pins: \
`rnx run --locked` and `rnx build --locked` re-verify every checksum before executing.\n\n\
## Offline-first rules (E113 family)\n\n\
Pinned versions never touch the network when a verified copy is cached; floating ranges need the \
registry. With an empty cache and no network, `@std/*` and registry imports fail as E108/E113 naming \
the registry instead of falling back silently. The binary carries no stdlib bytes: seed with \
`rnx download`, point `RNX_VENDOR_DIR` at a staged tree, or (for distributors) pre-seed `RNX_CACHE_HOME`. \
Over MCP, use `package_resolve` to check cache state, `doctor` for stdlib health, and `cache_status` \
for pool sizes; the E113 hint naming `rnx download` is a CLI step because cache writes stay out of MCP.\n\n\
## Security and publishing\n\n\
`rnx audit` prints the tier badge, capability list, call traces, lockfile drift, licenses, and the \
registry advisory verdict (fail-soft: an unreachable feed warns, a bad signature fails closed). \
Publishing (`rnx publish`) is a human-gated CLI step and is intentionally unavailable over MCP.";

const RESOURCE_GRAMMAR: &str = "rasmalai://spec/grammar";
const RESOURCE_ARCHITECTURE: &str = "rasmalai://spec/architecture";
const RESOURCE_MANIFEST: &str = "rasmalai://spec/manifest";
const RESOURCE_PACKAGES: &str = "rasmalai://spec/packages";
const RESOURCE_STDLIB_API: &str = "rasmalai://stdlib/api.json";
const RESOURCE_STDLIB_PREFIX: &str = "rasmalai://stdlib/";
const RESOURCE_STDLIB_SUFFIX: &str = ".json";

static STDLIB_API: OnceLock<Result<serde_json::Value, String>> = OnceLock::new();

fn load_stdlib_api() -> Result<serde_json::Value, String> {
    let mut candidates: Vec<String> = Vec::new();
    if let Ok(p) = std::env::var("RNX_STDLIB_API") {
        candidates.push(p);
    }
    candidates.push(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../website/static/data/api.json"
    )
    .to_string());
    candidates.push("website/static/data/api.json".to_string());
    candidates.push("../website/static/data/api.json".to_string());
    for path in &candidates {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                return serde_json::from_str(&text)
                    .map_err(|e| format!("cannot parse `{path}`: {e}"));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("cannot read `{path}`: {e}")),
        }
    }
    Err("stdlib api.json not found; set RNX_STDLIB_API or regenerate it with \
        `cargo run -p cli -- doc --json --stdlib`"
        .to_string())
}

fn stdlib_api() -> &'static Result<serde_json::Value, String> {
    STDLIB_API.get_or_init(load_stdlib_api)
}

fn stdlib_module_names(api: &serde_json::Value) -> Vec<String> {
    let empty = Vec::new();
    let modules = api.get("modules").and_then(|m| m.as_array()).unwrap_or(&empty);
    let mut names: Vec<String> = modules
        .iter()
        .filter_map(|m| m.get("name").and_then(|n| n.as_str()))
        .map(|n| n.to_string())
        .collect();
    names.sort();
    names.dedup();
    names
}

fn lookup_symbol(api: &serde_json::Value, query: &str, module: Option<&str>, limit: usize) -> String {
    let q = query.to_lowercase();
    let want_module = module.map(|m| {
        let m = m.strip_prefix("@std/").unwrap_or(m);
        m.strip_prefix("@std").unwrap_or(m).to_string()
    });
    let mut hits: Vec<(u8, String)> = Vec::new();
    let push = |hits: &mut Vec<(u8, String)>, score: u8, text: String| {
        if score > 0 || text.to_lowercase().contains(&q) {
            hits.push((score, text));
        }
    };
    let name_score = |name: &str| -> u8 {
        let n = name.to_lowercase();
        if n == q { 3 } else if n.starts_with(&q) { 2 } else if n.contains(&q) { 1 } else { 0 }
    };
    let doc_text = |docs: &serde_json::Value| -> String {
        docs.get("description").and_then(|d| d.as_str()).unwrap_or("").to_string()
    };
    let example_block = |docs: &serde_json::Value| -> String {
        docs.get("tags")
            .and_then(|t| t.as_array())
            .map(|tags| {
                tags.iter()
                    .filter(|t| t.get("kind").and_then(|k| k.as_str()) == Some("example"))
                    .filter_map(|t| t.get("text").and_then(|x| x.as_str()))
                    .map(|x| format!("```rasmalai\n{x}\n```"))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default()
    };
    let render = |fqn: &str, kind: &str, module: &str, sig: &str, docs: &serde_json::Value| -> String {
        let mut out = format!("## `{fqn}` ({kind}, @{module})\n");
        if !sig.is_empty() {
            out.push_str(&format!("`{sig}`\n"));
        }
        let desc = doc_text(docs);
        if !desc.is_empty() {
            out.push_str(&format!("{desc}\n"));
        }
        let ex = example_block(docs);
        if !ex.is_empty() {
            out.push_str(&format!("{ex}\n"));
        }
        out
    };
    let empty = Vec::new();
    let modules = api.get("modules").and_then(|m| m.as_array()).unwrap_or(&empty);
    for m in modules {
        let mod_name = m.get("name").and_then(|n| n.as_str()).unwrap_or("");
        if want_module.as_deref().is_some_and(|want| mod_name != want) {
            continue;
        }
        let module_path = format!("std/{mod_name}");
        for c in m.get("classes").and_then(|c| c.as_array()).unwrap_or(&empty) {
            let cls = c.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let docs = c.get("docs").unwrap_or(&serde_json::Value::Null);
            let fqn = cls.to_string();
            let mut score = name_score(cls).max(name_score(&format!("{mod_name}.{cls}")));
            if doc_text(docs).to_lowercase().contains(&q) {
                score = score.max(1);
            }
            if score > 0 {
                let init = c.get("init").and_then(|i| i.as_str()).unwrap_or("");
                push(&mut hits, score, render(&fqn, "class", &module_path, init, docs));
            }
            for f in c.get("fields").and_then(|f| f.as_array()).unwrap_or(&empty) {
                let fname = f.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let score = name_score(fname).max(name_score(&format!("{cls}.{fname}")));
                if score > 0 {
                    let ty = f.get("ty").and_then(|t| t.as_str()).unwrap_or("");
                    let fdocs = f.get("docs").unwrap_or(&serde_json::Value::Null);
                    push(&mut hits, score, render(&format!("{cls}.{fname}"), "field", &module_path, &format!("{fname}: {ty}"), fdocs));
                }
            }
            for meth in c.get("methods").and_then(|x| x.as_array()).unwrap_or(&empty) {
                let mname = meth.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let mdocs = meth.get("docs").unwrap_or(&serde_json::Value::Null);
                let mut score = name_score(mname).max(name_score(&format!("{cls}.{mname}")));
                if doc_text(mdocs).to_lowercase().contains(&q) {
                    score = score.max(1);
                }
                if score > 0 {
                    let sig = meth.get("sig").and_then(|s| s.as_str()).unwrap_or("");
                    push(&mut hits, score, render(&format!("{cls}.{mname}"), "method", &module_path, sig, mdocs));
                }
            }
        }
        for f in m.get("functions").and_then(|f| f.as_array()).unwrap_or(&empty) {
            let fname = f.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let fdocs = f.get("docs").unwrap_or(&serde_json::Value::Null);
            let mut score = name_score(fname);
            if doc_text(fdocs).to_lowercase().contains(&q) {
                score = score.max(1);
            }
            if score > 0 {
                let sig = f.get("sig").and_then(|s| s.as_str()).unwrap_or("");
                push(&mut hits, score, render(fname, "function", &module_path, sig, fdocs));
            }
        }
        for e in m.get("enums").and_then(|e| e.as_array()).unwrap_or(&empty) {
            let ename = e.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let edocs = e.get("docs").unwrap_or(&serde_json::Value::Null);
            if name_score(ename) > 0 {
                push(&mut hits, name_score(ename), render(ename, "enum", &module_path, "", edocs));
            }
            for v in e.get("variants").and_then(|v| v.as_array()).unwrap_or(&empty) {
                let vname = v.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let score = name_score(vname).max(name_score(&format!("{ename}.{vname}")));
                if score > 0 {
                    let vdocs = v.get("docs").unwrap_or(&serde_json::Value::Null);
                    push(&mut hits, score, render(&format!("{ename}.{vname}"), "enum variant", &module_path, "", vdocs));
                }
            }
        }
        for k in m.get("constants").and_then(|c| c.as_array()).unwrap_or(&empty) {
            let kname = k.get("name").and_then(|n| n.as_str()).unwrap_or("");
            if name_score(kname) > 0 {
                let kdocs = k.get("docs").unwrap_or(&serde_json::Value::Null);
                let sig = k.get("sig").and_then(|s| s.as_str()).unwrap_or("");
                push(&mut hits, name_score(kname), render(kname, "constant", &module_path, sig, kdocs));
            }
        }
    }
    hits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    hits.truncate(limit.max(1));
    if hits.is_empty() {
        let available: Vec<&str> = modules
            .iter()
            .filter_map(|m| m.get("name").and_then(|n| n.as_str()))
            .collect();
        return format!(
            "no symbols matching `{query}`{}. available modules: {}",
            match module {
                Some(m) => format!(" in `{m}`"),
                None => String::new(),
            },
            available.join(", ")
        );
    }
    let total = hits.len();
    let mut out = format!("{total} match(es) for `{query}`:\n\n");
    out.push_str(&hits.into_iter().map(|(_, t)| t).collect::<Vec<_>>().join("\n---\n"));
    out
}

fn truncate_text(text: &str, cap: usize) -> (String, bool) {
    if text.len() <= cap {
        return (text.to_string(), false);
    }
    let mut end = cap;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    (format!("{}... [truncated {} of {} bytes]", &text[..end], text.len() - end, text.len()), true)
}

fn span_view(src: &str, span: Option<diagnostics::Span>) -> (usize, usize, usize, usize, String) {
    let Some(span) = span else {
        return (0, 0, 0, 0, String::new());
    };
    let (line, col) = diagnostics::line_col(src, span.start);
    let (end_line, end_col) = diagnostics::line_col(src, span.end);
    let excerpt = src
        .lines()
        .nth(line.saturating_sub(1))
        .unwrap_or("")
        .chars()
        .take(240)
        .collect::<String>();
    (line, col, end_line, end_col, excerpt)
}

fn snippet_diag(src: &str, d: &diagnostics::Diagnostic, severity: &str) -> serde_json::Value {
    let (line, col, end_line, end_col, excerpt) = span_view(src, d.span);
    let mut item = serde_json::json!({
        "code": d.code.as_str(),
        "severity": severity,
        "message": d.message,
        "line": line,
        "col": col,
        "end_line": end_line,
        "end_col": end_col,
        "excerpt": excerpt,
    });
    if let Some(hint) = d.hint.as_ref() {
        item["hint"] = serde_json::Value::String(hint.clone());
    }
    item
}

fn cap_items(mut items: Vec<serde_json::Value>) -> (Vec<serde_json::Value>, usize) {
    if items.len() <= DIAG_CAP {
        return (items, 0);
    }
    let dropped = items.len() - DIAG_CAP;
    items.truncate(DIAG_CAP);
    (items, dropped)
}

fn snippet_envelope(src: &str, report: &crate::CheckReport) -> String {
    let errors: Vec<serde_json::Value> = report
        .errors
        .iter()
        .map(|d| snippet_diag(src, d, "error"))
        .collect();
    let warnings: Vec<serde_json::Value> = report
        .warnings
        .iter()
        .map(|d| snippet_diag(src, d, "warning"))
        .collect();
    let (errors, dropped_errors) = cap_items(errors);
    let (warnings, dropped_warnings) = cap_items(warnings);
    let status = if errors.is_empty() { "ok" } else { "error" };
    serde_json::json!({
        "status": status,
        "errors": errors,
        "warnings": warnings,
        "dropped_errors": dropped_errors,
        "dropped_warnings": dropped_warnings,
    })
    .to_string()
}

fn reshape_cli_json(stdout: &str) -> String {
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(stdout);
    let Ok(items) = parsed else {
        let (text, _) = truncate_text(stdout, TEXT_CAP);
        return serde_json::json!({ "status": "error", "errors": [], "warnings": [], "raw": text }).to_string();
    };
    let empty = Vec::new();
    let list = items.as_array().unwrap_or(&empty);
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    for item in list {
        let severity = item.get("severity").and_then(|s| s.as_str()).unwrap_or("error");
        if severity == "warning" {
            warnings.push(item.clone());
        } else {
            errors.push(item.clone());
        }
    }
    let (errors, dropped_errors) = cap_items(errors);
    let (warnings, dropped_warnings) = cap_items(warnings);
    let status = if errors.is_empty() { "ok" } else { "error" };
    serde_json::json!({
        "status": status,
        "errors": errors,
        "warnings": warnings,
        "dropped_errors": dropped_errors,
        "dropped_warnings": dropped_warnings,
    })
    .to_string()
}

async fn blocking_task<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("background task failed: {e}"))
}

async fn check_path_envelope(path: &Path) -> Result<String, String> {
    if !path.exists() {
        return Err(format!("cannot check `{}`: no such file or directory", path.display()));
    }
    let owned = path.to_path_buf();
    let outcome = blocking_task(move || {
        crate::check::run_check(std::slice::from_ref(&owned), None, true, false, true)
    })
    .await?;
    if outcome.stdout.trim().is_empty() {
        let (text, _) = truncate_text(&outcome.stderr, TEXT_CAP);
        return Err(format!("check produced no JSON for `{}`: {text}", path.display()));
    }
    Ok(reshape_cli_json(&outcome.stdout))
}

async fn check_path_pretty(path: &Path) -> Result<String, String> {
    if !path.exists() {
        return Err(format!("cannot check `{}`: no such file or directory", path.display()));
    }
    let owned = path.to_path_buf();
    blocking_task(move || {
        let report = crate::check_files(&owned.to_string_lossy());
        if report.errors.is_empty() && report.warnings.is_empty() {
            return Ok("ok: no errors, no warnings".to_string());
        }
        let theme = diagnostics::theme::AuraTheme::plain();
        let mut out = crate::report::render_compile_errors(&theme, &owned, &report.errors);
        for w in &report.warnings {
            out.push_str(&format!("warning[{}]: {}\n", w.code.as_str(), w.message));
        }
        Ok(out)
    })
    .await?
}

fn render_snippet_pretty(src: &str, report: &crate::CheckReport) -> String {
    if report.errors.is_empty() && report.warnings.is_empty() {
        return "ok: no errors, no warnings".to_string();
    }
    static PRETTY_COUNTER: AtomicUsize = AtomicUsize::new(0);
    let tmp = std::env::temp_dir().join(format!(
        "rnx-mcp-check-{}-{}.rnx",
        std::process::id(),
        PRETTY_COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    if std::fs::write(&tmp, src).is_err() {
        return snippet_envelope(src, report);
    }
    let theme = diagnostics::theme::AuraTheme::plain();
    let mut out = crate::report::render_compile_errors(&theme, &tmp, &report.errors);
    for w in &report.warnings {
        out.push_str(&format!("warning[{}]: {}\n", w.code.as_str(), w.message));
    }
    let _ = std::fs::remove_file(&tmp);
    out
}

#[derive(Default)]
struct ExecSlot {
    active: AtomicUsize,
    abandoned: AtomicUsize,
}

struct ExecGuard {
    slot: Arc<ExecSlot>,
    detached: bool,
}

impl ExecSlot {
    fn try_begin(self: &Arc<Self>) -> Result<ExecGuard, String> {
        if self.active.load(Ordering::SeqCst) > 0 {
            return Err("another run or build is still executing; wait for it to finish before starting a new one".to_string());
        }
        self.active.store(1, Ordering::SeqCst);
        Ok(ExecGuard { slot: Arc::clone(self), detached: false })
    }

    fn abandoned(&self) -> usize {
        self.abandoned.load(Ordering::SeqCst)
    }
}

impl Drop for ExecGuard {
    fn drop(&mut self) {
        if !self.detached {
            self.slot.active.store(0, Ordering::SeqCst);
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct CheckInput {
    /// Rasmalai source text. Exactly one of `source` or `path` is required.
    source: Option<String>,
    /// Path to a `.rnx` file or project directory, resolved project-aware (relative imports work).
    path: Option<String>,
    /// Return human-readable diagnostics instead of the default structured JSON.
    pretty: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RunInput {
    /// Rasmalai source text. Exactly one of `source` or `path` is required.
    source: Option<String>,
    /// Path to a `.rnx` file, run project-aware (relative imports resolve against the file).
    path: Option<String>,
    /// Entry function name. Defaults to `Main`.
    entry: Option<String>,
    /// Cap on captured stdout plus the return value, in bytes. Defaults to 32768, max 262144.
    max_output_bytes: Option<u64>,
    /// Return the legacy human-readable rendering instead of the default structured JSON.
    pretty: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct FmtInput {
    /// Rasmalai source text. Exactly one of `source` or `path` is required.
    source: Option<String>,
    /// Path to a `.rnx` file or directory. Default mode rewrites files in place.
    path: Option<String>,
    /// Report whether files are formatted without writing anything.
    check: Option<bool>,
    /// Show a unified diff instead of writing (path mode only).
    diff: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ExplainInput {
    /// Diagnostic code such as `E108`, `E303`, or `W104` (case-insensitive).
    code: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct LookupInput {
    /// Symbol or keyword to search (e.g. `ByteBuffer`, `AtomicInt`, `File.read`, `Process`).
    query: String,
    /// Optional standard module filter (e.g. `@std/bytes`, `@std/sync`).
    module: Option<String>,
    /// Max matches to return (1-20, default 8). Keep small to save tokens.
    limit: Option<u8>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct AuditInput {
    /// Path to the package directory to analyze. Defaults to the current project directory.
    path: Option<String>,
    /// Include the license graph report (like `rnx audit --licenses`).
    licenses: Option<bool>,
    /// Deny these SPDX license ids (like `rnx audit --deny-license`).
    deny_licenses: Option<Vec<String>>,
    /// Return the minimal publishing manifest instead of the audit (like `--export-manifest`).
    export_manifest: Option<bool>,
    /// Check locked deps against the registry advisory feed (fail-soft offline). Defaults to true.
    include_advisories: Option<bool>,
    /// Registry URL for advisories (or RNX_REGISTRY). Defaults to the toolchain default.
    registry: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct BuildInput {
    /// Rasmalai source text. Exactly one of `source` or `path` is required.
    source: Option<String>,
    /// Path to a `.rnx` entry file, built project-aware (relative imports resolve against the file).
    path: Option<String>,
    /// Entry function name. Defaults to `Main`.
    entry: Option<String>,
    /// Release mode (LLVM -O3, stripped). Slow; explicit opt-in only.
    release: Option<bool>,
    /// Optimization level, 0 or 1. Defaults to 1.
    opt_level: Option<u8>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct TestInput {
    /// Path to a project directory or `.rnx` entry file. Defaults to the server working directory.
    path: Option<String>,
    /// Run only tests whose names contain this substring.
    filter: Option<String>,
    /// Backend: `interpreter`, `cranelift` (default, matches `rnx test`), or `llvm`.
    backend: Option<String>,
    /// Match the filter as an exact test name.
    exact: Option<bool>,
    /// Cap on captured output, in bytes. Defaults to 32768, max 262144.
    max_output_bytes: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct LintInput {
    /// Path to a `.rnx` file, directory, or project. Defaults to the server working directory.
    path: Option<String>,
    /// Workspace member name (like `rnx lint -p`).
    package: Option<String>,
    /// Exit non-zero when any finding exists (still returns the findings).
    deny_warnings: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DocInput {
    /// Path to a project directory. Defaults to the server working directory.
    path: Option<String>,
    /// Workspace member name (like `rnx doc -p`).
    package: Option<String>,
    /// Skip path dependencies (like `rnx doc --no-deps`).
    no_deps: Option<bool>,
    /// Include private items (like `rnx doc --all`).
    include_private: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DoctorInput {
    /// Path used to locate the enclosing project for per-dependency status. Defaults to the server working directory.
    path: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct CacheInput {
    /// Path used to locate the enclosing project cache. Defaults to the server working directory.
    path: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct PackageInput {
    /// Package name (e.g. `httpkit`, `@rovelstars/ui`).
    name: String,
    /// Version requirement (e.g. `^3.45.0`, `*`). Defaults to `*`.
    range: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct EvalInput {
    /// Rasmalai expression or declaration, evaluated in a persistent JIT session. `:type <expr>` prints the static type; `:reset` clears state.
    code: String,
    /// Clear all session state before evaluating.
    reset: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ReloadInput {
    /// Optional specific file path to re-check. The whole program rebuilds regardless.
    file: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DiagPathInput {
    /// File path to typecheck (project-aware). Exactly one of `path` or `source` is required.
    path: Option<String>,
    /// Rasmalai source text (self-contained snippet). Exactly one of `path` or `source` is required.
    source: Option<String>,
}

#[derive(Clone)]
pub struct DevLink {
    tx: mpsc::Sender<DevCommand>,
}

impl DevLink {
    pub fn new(tx: mpsc::Sender<DevCommand>) -> Self {
        DevLink { tx }
    }
}

#[derive(Clone)]
pub struct RnxMcp {
    session: Arc<Mutex<ReplSession>>,
    dev: Option<DevLink>,
    exec: Arc<ExecSlot>,
}

impl RnxMcp {
    pub fn new() -> Self {
        RnxMcp {
            session: Arc::new(Mutex::new(ReplSession::new())),
            dev: None,
            exec: Arc::new(ExecSlot::default()),
        }
    }

    pub fn with_dev(link: DevLink) -> Self {
        RnxMcp {
            session: Arc::new(Mutex::new(ReplSession::new())),
            dev: Some(link),
            exec: Arc::new(ExecSlot::default()),
        }
    }
}

impl Default for RnxMcp {
    fn default() -> Self {
        Self::new()
    }
}

fn eval_in_session(session: &Mutex<ReplSession>, code: &str, reset: bool) -> (Vec<String>, bool) {
    let mut guard = match session.lock() {
        Ok(g) => g,
        Err(poison) => {
            let mut fresh = ReplSession::new();
            std::mem::swap(&mut *poison.into_inner(), &mut fresh);
            match session.lock() {
                Ok(g) => g,
                Err(_) => return (vec!["repl session unusable; reconnect".to_string()], true),
            }
        }
    };
    if reset {
        *guard = ReplSession::new();
    }
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| guard.eval(code)));
    match outcome {
        Ok(ReplOut::Exit) => (vec!["session persists; use :reset to clear state".to_string()], false),
        Ok(ReplOut::Lines(lines)) => {
            let is_error = lines.iter().any(|l| l.contains("error["));
            (lines, is_error)
        }
        Err(_) => {
            let fresh = ReplSession::new();
            *guard = fresh;
            (
                vec!["eval panicked; session was reset to a clean state".to_string()],
                true,
            )
        }
    }
}

fn text_result(text: String, is_error: bool) -> CallToolResult {
    if is_error {
        CallToolResult::error(vec![ContentBlock::text(text)])
    } else {
        CallToolResult::success(vec![ContentBlock::text(text)])
    }
}

fn output_cap(requested: Option<u64>) -> usize {
    let cap = requested.unwrap_or(OUTPUT_CAP).max(1).min(OUTPUT_MAX);
    cap as usize
}

fn cap_lines(lines: &[String], cap: usize) -> (Vec<String>, bool, usize) {
    let mut kept = Vec::new();
    let mut used = 0usize;
    for (index, line) in lines.iter().enumerate() {
        let piece = line.strip_suffix('\n').unwrap_or(line);
        if used + piece.len() + 1 > cap {
            if kept.is_empty() {
                let mut end = cap.min(piece.len());
                while end > 0 && !piece.is_char_boundary(end) {
                    end -= 1;
                }
                kept.push(piece[..end].to_string());
            }
            return (kept, true, lines.len() - index);
        }
        used += piece.len() + 1;
        kept.push(piece.to_string());
    }
    (kept, false, 0)
}

fn run_value_json(
    result: crate::RunResult,
    cap: usize,
    abandoned: usize,
) -> serde_json::Value {
    let (stdout_lines, truncated_stdout, dropped_stdout) = cap_lines(&result.output, cap);
    let (status, value, message) = match &result.outcome {
        RunOutcome::Value(v) => ("value", Some(v.display()), None),
        RunOutcome::Thrown(v) => ("thrown", None, Some(v.display())),
        RunOutcome::Uncaught { message, .. } => ("uncaught", None, Some(message.clone())),
        RunOutcome::Fatal { message, .. } => ("fatal", None, Some(message.clone())),
        RunOutcome::Compile(_) => ("compile_error", None, None),
    };
    let mut out = serde_json::json!({
        "status": status,
        "stdout_lines": stdout_lines,
        "truncated_stdout": truncated_stdout,
        "dropped_stdout_lines": dropped_stdout,
        "abandoned_workers": abandoned,
    });
    if let Some(v) = value {
        let (capped, cut) = truncate_text(&v, cap);
        out["value"] = serde_json::Value::String(capped);
        out["truncated_value"] = serde_json::Value::Bool(cut);
    }
    if status == "compile_error" {
        if let RunOutcome::Compile(errs) = &result.outcome {
            let items: Vec<serde_json::Value> = errs
                .iter()
                .map(|d| {
                    let mut item = serde_json::json!({
                        "code": d.code.as_str(),
                        "severity": "error",
                        "message": d.message,
                    });
                    if let Some(hint) = d.hint.as_ref() {
                        item["hint"] = serde_json::Value::String(hint.clone());
                    }
                    item
                })
                .collect();
            let (items, dropped) = cap_items(items);
            out["errors"] = serde_json::Value::Array(items);
            out["dropped_errors"] = serde_json::Value::from(dropped);
        }
    }
    if let Some(m) = message {
        let (capped, cut) = truncate_text(&m, cap);
        out["message"] = serde_json::Value::String(capped);
        out["truncated_message"] = serde_json::Value::Bool(cut);
    }
    out
}

fn run_pretty(result: crate::RunResult) -> String {
    let mut out: String = result
        .output
        .iter()
        .map(|s| s.strip_suffix('\n').unwrap_or(s))
        .collect::<Vec<_>>()
        .join("\n");
    match result.outcome {
        RunOutcome::Value(v) => {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("=> {}", v.display()));
        }
        RunOutcome::Thrown(v) => {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("thrown: {}", v.display()));
        }
        RunOutcome::Uncaught { message, .. } => {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("Uncaught exception: {message}"));
        }
        RunOutcome::Fatal { message, .. } => {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("fatal: {message}"));
        }
        RunOutcome::Compile(errs) => {
            let theme = diagnostics::theme::AuraTheme::plain();
            out = crate::report::render_compile_errors(&theme, Path::new("input.rnx"), &errs);
        }
    }
    out
}

fn run_snippet(src: String, entry: String, timeout: Duration, slot: Arc<ExecSlot>) -> Result<crate::RunResult, String> {
    let mut guard = slot.try_begin()?;
    let abandoned_handle = Arc::clone(&slot);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = crate::run_source(&src, &entry, Vec::new());
        let _ = tx.send(result);
        abandoned_handle.active.store(0, Ordering::SeqCst);
    });
    match rx.recv_timeout(timeout) {
        Ok(result) => Ok(result),
        Err(_) => {
            slot.abandoned.fetch_add(1, Ordering::SeqCst);
            guard.detached = true;
            let total = slot.abandoned();
            Err(format!(
                "run exceeded the {}s limit and its worker was abandoned ({} abandoned total); \
                wait for the worker to exit before starting another run or build, or restart `rnx mcp`",
                timeout.as_secs(),
                total
            ))
        }
    }
}

fn run_path_program(path: String, entry: String, timeout: Duration, slot: Arc<ExecSlot>) -> Result<crate::RunResult, String> {
    if !Path::new(&path).exists() {
        return Err(format!("cannot run `{path}`: no such file or directory"));
    }
    let mut guard = slot.try_begin()?;
    let abandoned_handle = Arc::clone(&slot);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = crate::run_files(&path, &entry, Vec::new());
        let _ = tx.send(result);
        abandoned_handle.active.store(0, Ordering::SeqCst);
    });
    match rx.recv_timeout(timeout) {
        Ok(result) => Ok(result),
        Err(_) => {
            slot.abandoned.fetch_add(1, Ordering::SeqCst);
            guard.detached = true;
            let total = slot.abandoned();
            Err(format!(
                "run exceeded the {}s limit and its worker was abandoned ({} abandoned total); \
                wait for the worker to exit before starting another run or build, or restart `rnx mcp`",
                timeout.as_secs(),
                total
            ))
        }
    }
}

#[tool_router]
impl RnxMcp {
    #[tool(
        name = "check",
        description = "Typecheck Rasmalai code project-aware (`path`) or self-contained (`source`). Returns structured JSON `{status, errors[], warnings[]}` with line/col/excerpt spans by default, or readable diagnostics with `pretty: true`. Prefer `get_diagnostics` for machine loops; `check` reports the same verdicts."
    )]
    async fn check(&self, params: Parameters<CheckInput>) -> Result<String, String> {
        let pretty = params.0.pretty.unwrap_or(false);
        match (&params.0.source, &params.0.path) {
            (Some(src), None) => {
                let owned = src.clone();
                let report = blocking_task(move || crate::check_source(&owned)).await?;
                if pretty {
                    Ok(render_snippet_pretty(src, &report))
                } else {
                    Ok(snippet_envelope(src, &report))
                }
            }
            (None, Some(path)) => {
                let p = Path::new(path);
                if pretty {
                    check_path_pretty(p).await
                } else {
                    check_path_envelope(p).await
                }
            }
            (Some(_), Some(_)) => Err("pass exactly one of `source` or `path`".to_string()),
            (None, None) => Err("pass `source` with Rasmalai code or `path` to a `.rnx` file or project directory".to_string()),
        }
    }

    #[tool(
        name = "run",
        description = "Typecheck and execute Rasmalai code in the interpreter (30s limit). `path` runs project-aware so relative imports resolve; `source` must be self-contained. Returns structured JSON `{status, stdout_lines[], value|message, truncated_*}` by default, or the legacy rendering with `pretty: true`. A timed-out worker is abandoned and blocks further runs until it exits."
    )]
    async fn run(&self, params: Parameters<RunInput>) -> Result<String, String> {
        let entry = params.0.entry.clone().unwrap_or_else(|| "Main".to_string());
        let cap = output_cap(params.0.max_output_bytes);
        let pretty = params.0.pretty.unwrap_or(false);
        let timeout = run_timeout();
        let slot = Arc::clone(&self.exec);
        let result = match (&params.0.source, &params.0.path) {
            (Some(src), None) => run_snippet(src.clone(), entry, timeout, slot)?,
            (None, Some(path)) => run_path_program(path.clone(), entry, timeout, slot)?,
            (Some(_), Some(_)) => return Err("pass exactly one of `source` or `path`".to_string()),
            (None, None) => return Err("pass `source` with Rasmalai code or `path` to a `.rnx` file".to_string()),
        };
        let abandoned = self.exec.abandoned();
        if pretty {
            Ok(run_pretty(result))
        } else {
            Ok(run_value_json(result, cap, abandoned).to_string())
        }
    }

    #[tool(
        name = "fmt",
        description = "Format Rasmalai code per the style specification. Snippet mode returns the formatted source (or a formatted/clean verdict with `check: true`). Path mode formats `.rnx` files in place by default; pass `check: true` for a read-only formatted check or `diff: true` for a unified diff."
    )]
    async fn fmt(&self, params: Parameters<FmtInput>) -> Result<String, String> {
        let check = params.0.check.unwrap_or(false);
        let diff = params.0.diff.unwrap_or(false);
        match (&params.0.source, &params.0.path) {
            (Some(src), None) => {
                if diff {
                    return Err("`diff` needs `path`; snippet mode returns the formatted source directly".to_string());
                }
                match frontend::fmt::format_source(src) {
                    Ok(formatted) if formatted == *src => Ok("already formatted".to_string()),
                    Ok(formatted) if check => {
                        let _ = formatted;
                        Ok("needs formatting".to_string())
                    }
                    Ok(formatted) => Ok(formatted),
                    Err(e) => Err(e),
                }
            }
            (None, Some(path)) => {
                let pb = Path::new(path);
                if !pb.exists() {
                    return Err(format!("cannot format `{path}`: no such file or directory"));
                }
                let outcome = crate::fmt::run_fmt(std::slice::from_ref(&pb.to_path_buf()), check, diff);
                let mut text = outcome.stdout;
                text.push_str(&outcome.stderr);
                let (capped, _) = truncate_text(&text, TEXT_CAP);
                if capped.is_empty() {
                    if check {
                        return Ok("already formatted".to_string());
                    }
                    return Ok("formatted".to_string());
                }
                Ok(capped)
            }
            (Some(_), Some(_)) => Err("pass exactly one of `source` or `path`".to_string()),
            (None, None) => Err("pass `source` with Rasmalai code or `path` to a `.rnx` file or directory".to_string()),
        }
    }

    #[tool(
        name = "explain",
        description = "Explain a Rasmalai diagnostic code (E108, E303, W104, ...). Returns the meaning and the fix hint."
    )]
    async fn explain(&self, params: Parameters<ExplainInput>) -> Result<String, String> {
        let code = params.0.code.trim().to_ascii_uppercase();
        match crate::explain(&code) {
            Some((title, fix)) => Ok(format!("{code}: {title}\nfix: {fix}")),
            None => Err(format!("unknown diagnostic code `{}`", params.0.code)),
        }
    }

    #[tool(
        name = "rasmalai_lookup_symbol",
        description = "Search the Rasmalai standard library (@std/*) for classes, methods, functions, interfaces, and types. Returns exact signatures, parameter types, and doc comments. Use this before guessing an @std/* API; it is cheaper than reading the stdlib catalog resource."
    )]
    async fn lookup_symbol(&self, params: Parameters<LookupInput>) -> Result<String, String> {
        let limit = params.0.limit.unwrap_or(8).clamp(1, 20) as usize;
        match stdlib_api() {
            Ok(api) => Ok(lookup_symbol(api, &params.0.query, params.0.module.as_deref(), limit)),
            Err(e) => Err(e.clone()),
        }
    }

    #[tool(
        name = "eval_code",
        description = "Evaluate a Rasmalai expression or declaration in a persistent JIT session (first eval pays JIT warmup). State carries across calls. `:type <expr>` prints the static type, `:reset` clears state (or pass `reset: true`). Code is capped at 32768 bytes to keep sessions responsive."
    )]
    async fn eval_code(&self, params: Parameters<EvalInput>) -> Result<CallToolResult, String> {
        if params.0.code.len() > EVAL_CODE_CAP {
            return Err(format!(
                "code is {} bytes, over the {EVAL_CODE_CAP} byte cap; split it into smaller evals",
                params.0.code.len()
            ));
        }
        let reset = params.0.reset.unwrap_or(false);
        let (lines, is_error) = eval_in_session(&self.session, &params.0.code, reset);
        Ok(text_result(lines.join("\n"), is_error))
    }

    #[tool(
        name = "hot_reload",
        description = "Force delta analysis and hot-swap updated functions in the running `rnx dev --mcp` session. Returns status (swapped, restarted, no_change, error), swapped function names, compile time, and diagnostics. Without an attached dev session it reports status error."
    )]
    async fn hot_reload(&self, params: Parameters<ReloadInput>) -> Result<CallToolResult, String> {
        let link = match &self.dev {
            Some(l) => l.clone(),
            None => {
                let report = HotReport {
                    status: "error".to_string(),
                    swapped_functions: Vec::new(),
                    compile_time_ms: 0,
                    diagnostics: vec![
                        "no dev session attached; start `rnx dev <file> --mcp` to enable hot reloads".to_string(),
                    ],
                };
                let text = serde_json::to_string_pretty(&report).unwrap_or_default();
                return Ok(text_result(text, true));
            }
        };
        let file = params.0.file.map(std::path::PathBuf::from);
        let (tx, rx) = mpsc::channel();
        link.tx
            .send(DevCommand::Rebuild { file, reply: tx })
            .map_err(|_| "dev watcher thread exited".to_string())?;
        match rx.recv_timeout(Duration::from_secs(120)) {
            Ok(report) => {
                let text = serde_json::to_string_pretty(&report).unwrap_or_default();
                Ok(text_result(text, report.status == "error"))
            }
            Err(_) => Ok(text_result(
                "hot reload timed out after 120s".to_string(),
                true,
            )),
        }
    }

    #[tool(
        name = "get_diagnostics",
        description = "Typecheck a file or source string without running it. Returns structured JSON `{status, errors[], warnings[]}` with code, message, byte span, line/col, excerpt, and hint. `path` is project-aware (relative imports resolve); `source` must be self-contained."
    )]
    async fn get_diagnostics(&self, params: Parameters<DiagPathInput>) -> Result<CallToolResult, String> {
        let text = match (&params.0.path, &params.0.source) {
            (Some(path), None) => check_path_envelope(Path::new(path)).await?,
            (None, Some(source)) => {
                let owned = source.clone();
                let report = blocking_task(move || crate::check_source(&owned)).await?;
                snippet_envelope(source, &report)
            }
            (Some(_), Some(_)) => return Err("pass exactly one of `path` or `source`".to_string()),
            (None, None) => return Err("pass `path` to a `.rnx` file or `source` with Rasmalai code".to_string()),
        };
        let is_error = text.contains("\"status\":\"error\"");
        Ok(text_result(text, is_error))
    }

    #[tool(
        name = "version",
        description = "Report the Rasmalai toolchain version served by this MCP server."
    )]
    async fn version(&self) -> Result<String, String> {
        Ok(format!("rnx {}", env!("CARGO_PKG_VERSION")))
    }

    #[tool(
        name = "inspect_package_capabilities",
        description = "Audit a package directory: capability surface (filesystem, network, exec, env, FFI), tier classification, and provenance chains, plus lockfile drift, license graph (`licenses`), and registry advisory verdicts (`include_advisories`, fail-soft offline). Read-only; advisory misses never fail the tool."
    )]
    async fn inspect_package_capabilities(
        &self,
        params: Parameters<AuditInput>,
    ) -> Result<String, String> {
        let dir = match &params.0.path {
            Some(p) => std::path::PathBuf::from(p),
            None => std::env::current_dir().map_err(|e| format!("cannot read cwd: {e}"))?,
        };
        if !dir.exists() {
            return Err(format!("cannot audit `{}`: no such file or directory", dir.display()));
        }
        if params.0.export_manifest.unwrap_or(false) {
            let report = crate::audit::audit_path(&dir).map_err(|e| format!("{e}"))?;
            return Ok(crate::audit::export_manifest(&report));
        }
        let report = crate::audit::audit_path(&dir).map_err(|e| format!("{e}"))?;
        let capabilities: serde_json::Value = serde_json::from_str(&crate::audit::report_json(&report))
            .unwrap_or(serde_json::Value::String(crate::audit::report_json(&report)));
        let drift = crate::audit::lockfile_drift(&dir);
        let denied = crate::audit::parse_deny_licenses(
            &params.0.deny_licenses.unwrap_or_default(),
        );
        for license in &denied {
            if !frontend::license::is_known_spdx(license) {
                return Err(format!("unknown license `{license}` in deny list (expected a known SPDX id)"));
            }
        }
        let (licenses_text, conflicts) = if params.0.licenses.unwrap_or(false) || !denied.is_empty() {
            match crate::audit::license_report_for_dir(&dir) {
                Ok(license_report) => {
                    let text = crate::audit::render_license_text(&license_report, &denied);
                    let conflicts = license_report.conflicts.clone();
                    let (capped, _) = truncate_text(&text, TEXT_CAP);
                    (Some(capped), conflicts)
                }
                Err(e) => return Err(format!("{e}")),
            }
        } else {
            (None, Vec::new())
        };
        let denied_hits: Vec<String> = match crate::audit::license_report_for_dir(&dir) {
            Ok(r) if params.0.licenses.unwrap_or(false) || !denied.is_empty() => {
                crate::audit::denied_hits(&r, &denied)
            }
            _ => Vec::new(),
        };
        let advisories = if params.0.include_advisories.unwrap_or(true) {
            let base = crate::audit::advisory_registry(params.0.registry.as_deref());
            let dir_owned = dir.clone();
            let verdict = blocking_task(move || crate::audit::audit_advisories(&dir_owned, &base)).await?;
            match verdict {
                crate::audit::AdvisoryCheck::Skipped(note) => {
                    serde_json::json!({ "status": "skipped", "note": note })
                }
                crate::audit::AdvisoryCheck::Rejected(reason) => {
                    serde_json::json!({ "status": "rejected", "note": reason })
                }
                crate::audit::AdvisoryCheck::Done(findings) => {
                    let text = crate::audit::render_advisory_text(&findings);
                    let (capped, _) = truncate_text(&text, TEXT_CAP);
                    serde_json::json!({ "status": "done", "findings": capped })
                }
            }
        } else {
            serde_json::json!({ "status": "disabled" })
        };
        Ok(serde_json::json!({
            "status": "ok",
            "path": dir.display().to_string(),
            "capabilities": capabilities,
            "drift": drift,
            "licenses": licenses_text,
            "denied": denied_hits,
            "conflicts": conflicts,
            "advisories": advisories,
        })
        .to_string())
    }

    #[tool(
        name = "build",
        description = "Typecheck and lower Rasmalai code to an LLVM object (kept in memory; use `rnx build` to link a binary). `path` builds project-aware. Returns `{status, bytes, entry, backend}` JSON. `release: true` runs LLVM -O3 and is slow: explicit opt-in only. Subject to the shared run/build execution slot and a 180s limit."
    )]
    async fn build(&self, params: Parameters<BuildInput>) -> Result<String, String> {
        let entry = params.0.entry.clone().unwrap_or_else(|| "Main".to_string());
        let release = params.0.release.unwrap_or(false);
        let opt_level = params.0.opt_level.unwrap_or(1);
        if opt_level > 1 {
            return Err(format!("bad `opt_level` {opt_level}: expected 0 or 1"));
        }
        let timeout = job_timeout();
        let slot = Arc::clone(&self.exec);
        let mut guard = slot.try_begin()?;
        let (tx, rx) = mpsc::channel();
        match (&params.0.source, &params.0.path) {
            (Some(src), None) => {
                let src = src.clone();
                let worker = Arc::clone(&slot);
                std::thread::spawn(move || {
                    let result = crate::build_object(&src, &entry, release);
                    let _ = tx.send((result.map_err(|e| (e, None)), entry, src));
                    worker.active.store(0, Ordering::SeqCst);
                });
            }
            (None, Some(path)) => {
                let path = path.clone();
                if !Path::new(&path).exists() {
                    return Err(format!("cannot build `{path}`: no such file or directory"));
                }
                let worker = Arc::clone(&slot);
                std::thread::spawn(move || {
                    let result = crate::build_files_cfg(&path, &entry, release, opt_level, None, &crate::CompileConfig::default());
                    let path_buf = Path::new(&path).to_path_buf();
                    let _ = tx.send((result.map_err(|e| (e, Some(path_buf))), entry, String::new()));
                    worker.active.store(0, Ordering::SeqCst);
                });
            }
            (Some(_), Some(_)) => return Err("pass exactly one of `source` or `path`".to_string()),
            (None, None) => return Err("pass `source` with Rasmalai code or `path` to a `.rnx` entry file".to_string()),
        };
        let (result, entry, src) = match rx.recv_timeout(timeout) {
            Ok((result, entry, src)) => (result, entry, src),
            Err(_) => {
                slot.abandoned.fetch_add(1, Ordering::SeqCst);
                guard.detached = true;
                return Err(format!(
                    "build exceeded the {}s limit and its worker was abandoned; wait for the worker to exit before starting another run or build, or restart `rnx mcp`",
                    timeout.as_secs()
                ));
            }
        };
        match result {
            Ok(bytes) => Ok(serde_json::json!({
                "status": "ok",
                "bytes": bytes.len(),
                "entry": entry,
                "backend": if release { "llvm -O3 (release)" } else { "llvm dev" },
                "note": "object kept in memory; use `rnx build` to link a binary",
            })
            .to_string()),
            Err((errs, file)) => {
                let items: Vec<serde_json::Value> = errs
                    .iter()
                    .map(|d| {
                        if file.is_none() && !src.is_empty() {
                            snippet_diag(&src, d, "error")
                        } else {
                            let mut item = serde_json::json!({
                                "code": d.code.as_str(),
                                "severity": "error",
                                "message": d.message,
                            });
                            if let Some(path) = d.file.as_ref() {
                                item["file"] = serde_json::Value::String(path.display().to_string());
                            }
                            if let Some(hint) = d.hint.as_ref() {
                                item["hint"] = serde_json::Value::String(hint.clone());
                            }
                            item
                        }
                    })
                    .collect();
                let (items, dropped) = cap_items(items);
                Ok(serde_json::json!({
                    "status": "compile_error",
                    "errors": items,
                    "dropped_errors": dropped,
                })
                .to_string())
            }
        }
    }

    #[tool(
        name = "test",
        description = "Run `test fn` blocks for a project (Cranelift JIT by default, like `rnx test`). `path` selects the project directory or entry file. Returns `{status, code, output}` JSON with capped output. Kills the worker past the 180s limit, so no state leaks."
    )]
    async fn test(&self, params: Parameters<TestInput>) -> Result<String, String> {
        let backend = params.0.backend.clone().unwrap_or_else(|| "cranelift".to_string());
        if !["interpreter", "cranelift", "llvm"].contains(&backend.as_str()) {
            return Err(format!("unknown backend `{backend}`: expected interpreter, cranelift, or llvm"));
        }
        let cap = output_cap(params.0.max_output_bytes);
        let dir = match &params.0.path {
            Some(p) => {
                let pb = Path::new(p);
                if !pb.exists() {
                    return Err(format!("cannot test `{p}`: no such file or directory"));
                }
                if pb.is_file() {
                    pb.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| Path::new(".").to_path_buf())
                } else {
                    pb.to_path_buf()
                }
            }
            None => std::env::current_dir().map_err(|e| format!("cannot read cwd: {e}"))?,
        };
        let exe = std::env::current_exe().map_err(|e| format!("cannot locate rnx binary: {e}"))?;
        let mut args = vec!["--no-color".to_string(), "test".to_string()];
        if let Some(filter) = &params.0.filter {
            args.push(filter.clone());
        }
        args.push("-O".to_string());
        args.push("1".to_string());
        args.push("--backend".to_string());
        args.push(backend);
        if params.0.exact.unwrap_or(false) {
            args.push("--exact".to_string());
        }
        let mut child = std::process::Command::new(exe)
            .args(&args)
            .current_dir(&dir)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot spawn test worker: {e}"))?;
        let deadline = Instant::now() + job_timeout();
        loop {
            match child.try_wait().map_err(|e| format!("cannot poll test worker: {e}"))? {
                Some(_) => break,
                None => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err("test exceeded the 180s limit and was killed".to_string());
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
            }
        }
        let output = child.wait_with_output().map_err(|e| format!("cannot read test output: {e}"))?;
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        if !output.stderr.is_empty() {
            text.push_str(&String::from_utf8_lossy(&output.stderr));
        }
        let (capped, truncated) = truncate_text(&text, cap);
        let code = output.status.code().unwrap_or(1);
        Ok(serde_json::json!({
            "status": if code == 0 { "pass" } else { "fail" },
            "code": code,
            "output": capped,
            "truncated": truncated,
        })
        .to_string())
    }

    #[tool(
        name = "lint",
        description = "Run the read-only static checks (L001 unused variable, L002 unused parameter, L003 unreachable code, L004 missing docs, L005 empty block) over a file, directory, or project. Returns findings as JSON. Never rewrites files."
    )]
    async fn lint(&self, params: Parameters<LintInput>) -> Result<String, String> {
        let root = match &params.0.path {
            Some(p) => std::path::PathBuf::from(p),
            None => std::env::current_dir().map_err(|e| format!("cannot read cwd: {e}"))?,
        };
        if !root.exists() {
            return Err(format!("cannot lint `{}`: no such file or directory", root.display()));
        }
        let outcome = crate::lint::run_lint(
            std::slice::from_ref(&root),
            params.0.package.as_deref(),
            false,
            true,
            params.0.deny_warnings.unwrap_or(false),
            false,
        );
        if !outcome.stderr.is_empty() && outcome.stdout.trim().is_empty() {
            return Err(outcome.stderr.trim().to_string());
        }
        let parsed: serde_json::Value =
            serde_json::from_str(outcome.stdout.trim()).unwrap_or(serde_json::Value::Null);
        let total = parsed.as_array().map(|a| a.len()).unwrap_or(0);
        let findings = if let Some(arr) = parsed.as_array() {
            let capped: Vec<serde_json::Value> = arr.iter().take(DIAG_CAP).cloned().collect();
            serde_json::Value::Array(capped)
        } else {
            parsed
        };
        Ok(serde_json::json!({
            "status": if total == 0 { "ok" } else { "findings" },
            "code": outcome.code,
            "findings_total": total,
            "findings_dropped": total.saturating_sub(DIAG_CAP),
            "findings": findings,
        })
        .to_string())
    }

    #[tool(
        name = "doc",
        description = "Collect the JSON API docs for a project (like `rnx doc --json`, written under `target/doc`). Returns module names, counts, and a capped preview. Read-only apart from the `target/doc` build artifact."
    )]
    async fn doc(&self, params: Parameters<DocInput>) -> Result<String, String> {
        let dir = match &params.0.path {
            Some(p) => std::path::PathBuf::from(p),
            None => std::env::current_dir().map_err(|e| format!("cannot read cwd: {e}"))?,
        };
        if !dir.exists() {
            return Err(format!("cannot document `{}`: no such file or directory", dir.display()));
        }
        let out_path = crate::generate_docs_json(
            &dir,
            params.0.package.as_deref(),
            params.0.no_deps.unwrap_or(false),
            params.0.include_private.unwrap_or(false),
        )
        .map_err(|e| e.message)?;
        let text = std::fs::read_to_string(&out_path)
            .map_err(|e| format!("cannot read {}: {e}", out_path.display()))?;
        let parsed: serde_json::Value =
            serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
        let modules = parsed
            .get("modules")
            .and_then(|m| m.as_array())
            .cloned()
            .or_else(|| parsed.as_array().cloned())
            .unwrap_or_default();
        let names: Vec<serde_json::Value> = modules
            .iter()
            .filter_map(|m| m.get("name").and_then(|n| n.as_str()))
            .map(|n| serde_json::Value::String(n.to_string()))
            .collect();
        let (preview, truncated) = truncate_text(&text, TEXT_CAP / 2);
        Ok(serde_json::json!({
            "status": "ok",
            "path": out_path.display().to_string(),
            "bytes": text.len(),
            "module_count": modules.len(),
            "modules": names,
            "preview": preview,
            "preview_truncated": truncated,
        })
        .to_string())
    }

    #[tool(
        name = "doctor",
        description = "Report toolchain cache health without repairing anything: stdlib pin and missing packages, precompiled product status, and per-dependency cached/absent state. Read-only; repair stays on the `rnx doctor --repair-std` CLI."
    )]
    async fn doctor(&self, params: Parameters<DoctorInput>) -> Result<String, String> {
        let cwd = match &params.0.path {
            Some(p) => std::path::PathBuf::from(p),
            None => std::env::current_dir().map_err(|e| format!("cannot read cwd: {e}"))?,
        };
        if !cwd.exists() {
            return Err(format!("cannot diagnose `{}`: no such file or directory", cwd.display()));
        }
        let stdlib = match frontend::stdlib_seed::read_std_pin() {
            None => serde_json::json!({
                "status": "missing",
                "note": "no pin file; run `rnx download` to seed the cache",
            }),
            Some(pin) => {
                let missing = frontend::stdlib_seed::missing_std_packages();
                if missing.is_empty() {
                    let mut status = serde_json::json!({
                        "status": "ok",
                        "packages": pin.packages.len(),
                    });
                    match frontend::products::std_products_summary() {
                        Some((files, pkgs)) => {
                            status["products"] = serde_json::json!({
                                "status": "precompiled",
                                "files": files,
                                "packages": pkgs,
                            });
                        }
                        _ => {
                            status["products"] = serde_json::json!({
                                "status": "absent",
                                "note": "run `rnx download` to precompile them",
                            });
                        }
                    }
                    status
                } else {
                    serde_json::json!({
                        "status": "incomplete",
                        "missing": missing,
                        "note": "run `rnx download` (or `rnx doctor --repair-std`)",
                    })
                }
            }
        };
        let (product_files, product_bytes) = frontend::products::products_usage();
        let root = frontend::project::find_project_root(&cwd).unwrap_or_else(|| cwd.clone());
        let deps: Vec<serde_json::Value> = frontend::modules::product_statuses(&root)
            .iter()
            .map(|s| {
                serde_json::json!({
                    "package": s.pkg,
                    "version": s.version,
                    "present": s.present,
                })
            })
            .collect();
        Ok(serde_json::json!({
            "status": "ok",
            "stdlib": stdlib,
            "products": { "files": product_files, "bytes": product_bytes },
            "product_objects_bytes": frontend::products::global_objects_usage_bytes(),
            "project_root": root.display().to_string(),
            "dependencies": deps,
        })
        .to_string())
    }

    #[tool(
        name = "cache_status",
        description = "Report cache pool sizes (global cache, content-addressed store, products, project cache) without pruning anything. Read-only; pruning stays on the `rnx cache prune` CLI."
    )]
    async fn cache_status(&self, params: Parameters<CacheInput>) -> Result<String, String> {
        let cwd = match &params.0.path {
            Some(p) => std::path::PathBuf::from(p),
            None => std::env::current_dir().map_err(|e| format!("cannot read cwd: {e}"))?,
        };
        let global = frontend::cache::global_cache_dir();
        let store = frontend::fetch::store_root();
        let (product_files, product_bytes) = frontend::products::products_usage();
        let project = frontend::project::find_project_root(&cwd).unwrap_or_else(|| cwd.clone());
        let project_cache = frontend::cache::project_cache_dir(&project);
        Ok(serde_json::json!({
            "status": "ok",
            "global": {
                "path": global.display().to_string(),
                "bytes": frontend::cache::cache_usage_bytes(&global),
            },
            "store": {
                "path": store.display().to_string(),
                "bytes": frontend::fetch::store_usage_bytes(),
            },
            "products": { "files": product_files, "bytes": product_bytes },
            "project": {
                "root": project.display().to_string(),
                "cache": project_cache.display().to_string(),
                "bytes": frontend::cache::cache_usage_bytes(&project_cache),
            },
        })
        .to_string())
    }

    #[tool(
        name = "package_resolve",
        description = "Resolve a registry package version from the local cache only (never touches the network). Returns the best cached match for the range plus the full cached version list, so agents can tell `cached` apart from `download-needed` before shelling out to `rnx download`."
    )]
    async fn package_resolve(&self, params: Parameters<PackageInput>) -> Result<String, String> {
        let name = params.0.name.trim().to_string();
        if name.is_empty() {
            return Err("pass a package `name` (e.g. `httpkit`)".to_string());
        }
        let range = params.0.range.unwrap_or_else(|| "*".to_string());
        if !frontend::fetch::is_valid_range(&range) {
            return Err(format!("bad version range `{range}`"));
        }
        let probe = frontend::fetch::probe_cache(&name, &range);
        let download_needed = probe.resolved.is_none();
        Ok(serde_json::json!({
            "status": "ok",
            "name": probe.requested,
            "range": probe.range,
            "resolved": probe.resolved,
            "cached_versions": probe.cached_versions,
            "engine_blocked": probe.engine_blocked,
            "download_needed": download_needed,
            "hint": if download_needed {
                "no cached version satisfies the range; run `rnx download` online to seed the cache"
            } else {
                "a verified cached copy satisfies the range; locked builds stay off the wire"
            },
        })
        .to_string())
    }
}

fn resource_entries() -> Vec<Resource> {
    let mut entries: Vec<(String, String, String)> = vec![
        (RESOURCE_GRAMMAR.to_string(), "Rasmalai grammar".to_string(), "Syntax, types, operators, imports, and control-flow rules.".to_string()),
        (RESOURCE_ARCHITECTURE.to_string(), "Rasmalai architecture".to_string(), "ARC memory model, GenRefs, and systems-language contrasts.".to_string()),
        (RESOURCE_MANIFEST.to_string(), "Project.config manifest".to_string(), "Manifest layout, SemVer deps, and @std imports.".to_string()),
        (RESOURCE_PACKAGES.to_string(), "Package management lifecycle".to_string(), "Add, lock, fetch, download, vendor, doctor, cache, audit, and offline E113 rules.".to_string()),
    ];
    match stdlib_api() {
        Ok(api) => {
            for name in stdlib_module_names(api) {
                entries.push((
                    format!("{RESOURCE_STDLIB_PREFIX}{name}{RESOURCE_STDLIB_SUFFIX}"),
                    format!("@std/{name} API"),
                    format!("Symbol catalog for @std/{name}. Prefer rasmalai_lookup_symbol for single-symbol questions."),
                ));
            }
            entries.push((
                RESOURCE_STDLIB_API.to_string(),
                "Rasmalai stdlib API".to_string(),
                "Complete @std/* symbol catalog as JSON (~336 KB; heavy: prefer rasmalai_lookup_symbol or a per-module resource).".to_string(),
            ));
        }
        Err(_) => {
            entries.push((
                RESOURCE_STDLIB_API.to_string(),
                "Rasmalai stdlib API".to_string(),
                "Complete @std/* symbol catalog as JSON (unavailable: regenerate with `cargo run -p cli -- doc --json --stdlib`).".to_string(),
            ));
        }
    }
    entries
        .into_iter()
        .map(|(uri, name, description)| {
            let mime = if uri.ends_with(".json") {
                "application/json"
            } else {
                "text/markdown"
            };
            Resource::new(uri, name)
                .with_description(description)
                .with_mime_type(mime)
        })
        .collect()
}

fn read_resource_text(uri: &str) -> Result<(String, String), McpError> {
    match uri {
        RESOURCE_GRAMMAR => Ok((GRAMMAR_SPEC.to_string(), "text/markdown".to_string())),
        RESOURCE_ARCHITECTURE => Ok((ARCHITECTURE_SPEC.to_string(), "text/markdown".to_string())),
        RESOURCE_MANIFEST => Ok((MANIFEST_SPEC.to_string(), "text/markdown".to_string())),
        RESOURCE_PACKAGES => Ok((PACKAGES_SPEC.to_string(), "text/markdown".to_string())),
        RESOURCE_STDLIB_API => {
            let path = std::env::var("RNX_STDLIB_API").unwrap_or_else(|_| {
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../website/static/data/api.json"
                )
                .to_string()
            });
            std::fs::read_to_string(&path)
                .map(|text| (text, "application/json".to_string()))
                .map_err(|e| McpError::internal_error(format!("cannot read stdlib api.json: {e}"), None))
        }
        _ => {
            if let Some(rest) = uri.strip_prefix(RESOURCE_STDLIB_PREFIX)
                && let Some(module) = rest.strip_suffix(RESOURCE_STDLIB_SUFFIX)
            {
                let api = stdlib_api()
                    .as_ref()
                    .map_err(|e| McpError::internal_error(e.clone(), None))?;
                let empty = Vec::new();
                let modules = api.get("modules").and_then(|m| m.as_array()).unwrap_or(&empty);
                for m in modules {
                    if m.get("name").and_then(|n| n.as_str()) == Some(module) {
                        let shard = serde_json::json!({ "modules": [m] }).to_string();
                        return Ok((shard, "application/json".to_string()));
                    }
                }
                return Err(McpError::resource_not_found(
                    format!("unknown stdlib module `{module}`; list resources or call rasmalai_lookup_symbol"),
                    None,
                ));
            }
            Err(McpError::resource_not_found(format!("unknown resource `{uri}`"), None))
        }
    }
}

fn expert_prompt_text() -> String {
    format!(
        "You are a Rasmalai expert. Core invariants: only `Int` and `Float` exist (no sized integers); \
`let` bindings are mutable by default (no `mut`); classes are built with `new` like `new Meter(20)` \
(a direct class call is E204) while structs and records use direct calls like `Point(1.0, 2.0)`; single-statement \
`if`/`else` bodies are valid but `let` inside them is E108; lone `;` is an \
empty statement; memory is deterministic scope-based ARC and cycles must use `GenRef` back-edges.\n\n\
Modules: each file is a namespace; `public` items import freely between project files. A minimal \
multi-file project looks like this (`src/main.rnx` plus `src/math.rnx`, entry defaults to `src/main.rnx`):\n\n\
```rnx\n// src/math.rnx\nfn double(x: Int): Int {{\n    return x * 2;\n}}\n```\n\n\
```rnx\n// src/main.rnx\nimport {{ double }} from \"./math\";\nimport {{ ByteBuffer }} from \"@std/bytes\";\n\nfn Main(): Int {{\n    return double(21);\n}}\n```\n\n\
Standard library: `@std/*` modules need no manifest entry and no version pin. Never guess an `@std/*` \
signature: call `rasmalai_lookup_symbol` first (it is cheaper than reading any catalog resource), and \
browse one module at a time with `rasmalai://stdlib/<module>.json` (for example \
`rasmalai://stdlib/bytes.json`) instead of the full 300 KB+ `rasmalai://stdlib/api.json` catalog.\n\n\
Projects: `Project.config` declares identity and deps; `rnx lock` writes `Project.deplock`; `--locked` \
builds verify pins. Read `rasmalai://spec/packages` before touching manifests, locks, or the cache: \
the cache is offline-first (E113 on a cold miss), and `package_resolve`, `doctor`, and `cache_status` \
diagnose without network while `rnx download` stays a CLI step.\n\n\
Loop: validate with `get_diagnostics` (structured JSON, machine-first) or `check` (`pretty: true` for \
readable output), execute with `run`, lint with `lint`, test with `test`, lower with `build`. \
Prefer `path` inputs for project files so relative imports resolve; `source` snippets must be \
self-contained. Always run `check` on generated code before answering. In `eval_code`, `:type <expr>` \
prints the static type and `:reset` (or `reset: true`) clears the session.\n\n\
Read these resources for full context: `{RESOURCE_GRAMMAR}` for syntax, `{RESOURCE_ARCHITECTURE}` \
for the memory model, `{RESOURCE_MANIFEST}` for manifests, and `{RESOURCE_PACKAGES}` for the \
package lifecycle."
    )
}

fn convert_prompt_text(source_language: &str, code: &str) -> String {
    format!(
        "Translate the following {source_language} code into idiomatic Rasmalai.\n\n\
Rosetta rules: map dynamic objects to Rasmalai records or structs; erase Rust lifetimes into \
lexical ARC ownership (shared back-edges become `GenRef`); replace Go channels and goroutines with \
`@std/sync` threadpools, mutexes, or barriers; replace sized integers with `Int`/`Float`; map \
constructors to `new Class(...)` for classes and direct calls for structs and records; keep `fn Main(): Int` as the entry point when producing a \
runnable program. Confirm every `@std/*` signature with `rasmalai_lookup_symbol` instead of guessing, \
then verify the result with the `check` tool before answering.\n\n\
```{source_language}\n{code}\n```"
    )
}

#[tool_handler]
impl rmcp::ServerHandler for RnxMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_prompts()
                .build(),
        )
        .with_server_info(Implementation::new("rnx-mcp", env!("CARGO_PKG_VERSION")))
        .with_instructions(INSTRUCTIONS)
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Owned(vec![ProtocolVersion::V_2024_11_05])
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        Ok(ListResourcesResult::with_all_items(resource_entries()))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        let (text, mime) = read_resource_text(&request.uri)?;
        Ok(ReadResourceResult::new(vec![
            ResourceContents::text(text, &request.uri).with_mime_type(mime),
        ])
        .into())
    }

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, McpError> {
        Ok(ListPromptsResult::with_all_items(vec![
            Prompt::new(
                "rasmalai-expert",
                Some("Bootstrap an AI agent with complete context on Rasmalai syntax, invariants, and compiler validation tools."),
                None,
            ),
            Prompt::new(
                "convert-to-rasmalai",
                Some("Translate code from Rust, Go, TypeScript, or C++ into idiomatic Rasmalai."),
                Some(vec![
                    PromptArgument::new("source_language")
                        .with_required(true)
                        .with_description("Source language: rust, go, typescript, or c++"),
                    PromptArgument::new("code")
                        .with_required(true)
                        .with_description("Source code to translate"),
                ]),
            ),
        ]))
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, McpError> {
        let args = request.arguments.clone().unwrap_or_default();
        let arg = |name: &str| args.get(name).and_then(|v| v.as_str()).unwrap_or("").to_string();
        match request.name.as_str() {
            "rasmalai-expert" => Ok(GetPromptResult::new(vec![PromptMessage::new_text(
                Role::User,
                expert_prompt_text(),
            )])
            .into()),
            "convert-to-rasmalai" => {
                let language = arg("source_language");
                let code = arg("code");
                if language.is_empty() || code.is_empty() {
                    return Err(McpError::invalid_params(
                        "convert-to-rasmalai requires `source_language` and `code` arguments",
                        None,
                    ));
                }
                Ok(GetPromptResult::new(vec![PromptMessage::new_text(
                    Role::User,
                    convert_prompt_text(&language, &code),
                )])
                .into())
            }
            other => Err(McpError::invalid_params(format!("unknown prompt `{other}`"), None)),
        }
    }
}

pub async fn serve() -> Result<(), String> {
    serve_with(RnxMcp::new()).await
}

pub async fn serve_with_dev(link: DevLink) -> Result<(), String> {
    serve_with(RnxMcp::with_dev(link)).await
}

async fn serve_with(server: RnxMcp) -> Result<(), String> {
    let transport = rmcp::transport::io::stdio();
    let service = rmcp::serve_server(server, transport)
        .await
        .map_err(|e| format!("mcp serve failed: {e}"))?;
    service
        .waiting()
        .await
        .map_err(|e| format!("mcp server exited: {e}"))?;
    Ok(())
}
