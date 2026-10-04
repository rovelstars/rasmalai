use std::borrow::Cow;
use std::path::Path;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

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

const INSTRUCTIONS: &str = "You are an expert compiler and software engineer specializing in Rasmalai. \
Rasmalai is a high-performance systems language with the ergonomics of a modern web language.\n\n\
Core Invariants:\n\
1. Unified 64-bit Numerics: Only `Int` and `Float` exist. There are NO sized integers (no `u8`, `i32`, `u64`).\n\
2. Bindings: Variables declared with `let` are mutable by default. There is NO `mut` keyword.\n\
3. Constructors: Classes use `new`: `new Meter(20)` allocates the object and runs `init` (calling a class directly, `Meter(20)`, is E204). Structs and records use direct calls: `Point(1.0, 2.0)` (`new` on a struct is an error).\n\
4. Control Flow: Single-statement `if` and `else` bodies are valid, but lexical declarations (`let`) inside single-statement bodies are forbidden (E108). Lone semicolons `;` are valid empty statements.\n\
5. Memory and Cycles: Memory is managed via deterministic scope-based ARC with write-dominance cleanup on loop back-edges. Cyclical data structures MUST NOT use raw pointer loops; use GenRefs (`GenRef.of(obj)`, stale `.get()` returns null).\n\
6. Tool Protocol: ALWAYS read `rasmalai://spec/grammar` if you are unsure of syntax. ALWAYS call the `check` tool to validate code before presenting it to the user.\n\n\
Entry point is `fn Main(): Int`. Tools operate on self-contained snippets; for multi-file projects use the `rnx` CLI directly.";

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
- `permissions` (optional): array of capability strings forming the security ceiling.\n\
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
Use `rnx lock` to write `Project.deplock` and `rnx vendor` to copy sources into `vendor/`.";

const RESOURCE_GRAMMAR: &str = "rasmalai://spec/grammar";
const RESOURCE_ARCHITECTURE: &str = "rasmalai://spec/architecture";
const RESOURCE_MANIFEST: &str = "rasmalai://spec/manifest";
const RESOURCE_STDLIB_API: &str = "rasmalai://stdlib/api.json";

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

fn lookup_symbol(api: &serde_json::Value, query: &str, module: Option<&str>) -> String {
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
    hits.truncate(8);
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

#[derive(Debug, Deserialize, JsonSchema)]
struct SourceInput {
    /// Rasmalai source text. Exactly one of `source` or `path` is required.
    source: Option<String>,
    /// Path to a `.rnx` file, read relative to the server working directory.
    path: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RunInput {
    /// Rasmalai source text. Exactly one of `source` or `path` is required.
    source: Option<String>,
    /// Path to a `.rnx` file, read relative to the server working directory.
    path: Option<String>,
    /// Entry function name. Defaults to `Main`.
    entry: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ExplainInput {
    /// Diagnostic code such as `E108`, `E303`, or `W104`.
    code: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct LookupInput {
    /// Symbol or keyword to search (e.g. `ByteBuffer`, `AtomicInt`, `File.read`, `Process`).
    query: String,
    /// Optional standard module filter (e.g. `@std/bytes`, `@std/sync`).
    module: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct AuditInput {
    /// Path to the package directory to analyze. Defaults to the current project directory.
    path: Option<String>,
}

fn resolve_source(input: &SourceInput) -> Result<String, String> {
    match (&input.source, &input.path) {
        (Some(src), None) => Ok(src.clone()),
        (None, Some(path)) => std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read `{path}`: {e}")),
        (Some(_), Some(_)) => Err("pass exactly one of `source` or `path`".to_string()),
        (None, None) => Err("pass `source` with Rasmalai code or `path` to a `.rnx` file".to_string()),
    }
}

fn render_check(src: &str) -> String {
    let report = crate::check_source(src);
    if report.errors.is_empty() && report.warnings.is_empty() {
        return "ok: no errors, no warnings".to_string();
    }
    let theme = diagnostics::theme::AuraTheme::plain();
    let mut out = String::new();
    if !report.errors.is_empty() {
        out.push_str(&crate::report::render_compile_errors(
            &theme,
            Path::new("input.rnx"),
            &report.errors,
        ));
    }
    for w in &report.warnings {
        out.push_str(&format!("warning[{}]: {}\n", w.code.as_str(), w.message));
    }
    out
}

#[derive(Debug, Deserialize, JsonSchema)]
struct EvalInput {
    /// Rasmalai expression or declaration, evaluated in a persistent JIT session.
    code: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ReloadInput {
    /// Optional specific file path to re-check. The whole program rebuilds regardless.
    file: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DiagPathInput {
    /// File path to typecheck. Exactly one of `path` or `source` is required.
    path: Option<String>,
    /// Rasmalai source text. Exactly one of `path` or `source` is required.
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
}

impl RnxMcp {
    pub fn new() -> Self {
        RnxMcp {
            session: Arc::new(Mutex::new(ReplSession::new())),
            dev: None,
        }
    }

    pub fn with_dev(link: DevLink) -> Self {
        RnxMcp {
            session: Arc::new(Mutex::new(ReplSession::new())),
            dev: Some(link),
        }
    }
}

impl Default for RnxMcp {
    fn default() -> Self {
        Self::new()
    }
}

fn eval_in_session(session: &Mutex<ReplSession>, code: &str) -> (Vec<String>, bool) {
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

#[tool_router]
impl RnxMcp {
    #[tool(
        name = "check",
        description = "Typecheck Rasmalai code without running it. Returns rendered diagnostics (error codes, spans, fix hints) or `ok`. Tools operate on self-contained snippets; for multi-file projects use the `rnx` CLI directly."
    )]
    async fn check(&self, params: Parameters<SourceInput>) -> Result<String, String> {
        let src = resolve_source(&params.0)?;
        Ok(render_check(&src))
    }

    #[tool(
        name = "run",
        description = "Typecheck and execute Rasmalai code in the interpreter (30s limit). Returns printed lines plus `=> value`, `thrown: ...`, or `fatal: ...`. Runs abandon their thread on timeout; prefer small snippets. Self-contained snippets only."
    )]
    async fn run(&self, params: Parameters<RunInput>) -> Result<String, String> {
        let inner = SourceInput {
            source: params.0.source.clone(),
            path: params.0.path.clone(),
        };
        let src = resolve_source(&inner)?;
        let entry = params.0.entry.clone().unwrap_or_else(|| "Main".to_string());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = crate::run_source(&src, &entry, Vec::new());
            let _ = tx.send(result);
        });
        let result = rx
            .recv_timeout(RUN_TIMEOUT)
            .map_err(|_| "run exceeded the 30s limit and was abandoned".to_string())?;
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
        Ok(out)
    }

    #[tool(
        name = "fmt",
        description = "Format Rasmalai code per the style specification. Returns the formatted source, or reports that input is already formatted."
    )]
    async fn fmt(&self, params: Parameters<SourceInput>) -> Result<String, String> {
        let src = resolve_source(&params.0)?;
        match frontend::fmt::format_source(&src) {
            Ok(formatted) if formatted == src => Ok("already formatted".to_string()),
            Ok(formatted) => Ok(formatted),
            Err(e) => Err(e),
        }
    }

    #[tool(
        name = "explain",
        description = "Explain a Rasmalai diagnostic code (E108, E303, W104, ...). Returns the meaning and the fix hint."
    )]
    async fn explain(&self, params: Parameters<ExplainInput>) -> Result<String, String> {
        match crate::explain(&params.0.code) {
            Some((title, fix)) => Ok(format!("{}: {title}\nfix: {fix}", params.0.code)),
            None => Err(format!("unknown diagnostic code `{}`", params.0.code)),
        }
    }

    #[tool(
        name = "rasmalai_lookup_symbol",
        description = "Search the Rasmalai standard library (@std/*) for classes, methods, functions, interfaces, and types. Returns exact signatures, parameter types, and doc comments."
    )]
    async fn lookup_symbol(&self, params: Parameters<LookupInput>) -> Result<String, String> {
        match stdlib_api() {
            Ok(api) => Ok(lookup_symbol(api, &params.0.query, params.0.module.as_deref())),
            Err(e) => Err(e.clone()),
        }
    }

    #[tool(
        name = "eval_code",
        description = "Evaluate a Rasmalai expression or declaration in a persistent JIT session. State (variables, functions) carries across calls. Returns the value or a declaration confirmation; diagnostics come back with isError."
    )]
    async fn eval_code(&self, params: Parameters<EvalInput>) -> Result<CallToolResult, String> {
        let (lines, is_error) = eval_in_session(&self.session, &params.0.code);
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
        description = "Typecheck a file or source string without running it. Returns structured JSON with error and warning arrays (code, message, byte span, hint)."
    )]
    async fn get_diagnostics(&self, params: Parameters<DiagPathInput>) -> Result<CallToolResult, String> {
        let src = match (&params.0.path, &params.0.source) {
            (Some(path), None) => std::fs::read_to_string(path)
                .map_err(|e| format!("cannot read `{path}`: {e}"))?,
            (None, Some(source)) => source.clone(),
            (Some(_), Some(_)) => return Err("pass exactly one of `path` or `source`".to_string()),
            (None, None) => return Err("pass `path` to a `.rnx` file or `source` with Rasmalai code".to_string()),
        };
        let report = crate::check_source(&src);
        let diag_json = |d: &diagnostics::Diagnostic| {
            serde_json::json!({
                "code": d.code.as_str(),
                "message": d.message,
                "span": d.span.as_ref().map(|s| serde_json::json!({ "start": s.start, "end": s.end })),
                "hint": d.hint,
            })
        };
        let out = serde_json::json!({
            "errors": report.errors.iter().map(diag_json).collect::<Vec<_>>(),
            "warnings": report.warnings.iter().map(diag_json).collect::<Vec<_>>(),
        });
        let text = serde_json::to_string_pretty(&out).unwrap_or_default();
        Ok(text_result(text, !report.errors.is_empty()))
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
        description = "Analyzes a Rasmalai package or local project directory for security capabilities (filesystem, network, process execution, environment variables, FFI/unsafe), returning the 4-tier security classification, required capability flags, and source-to-sink provenance call chains."
    )]
    async fn inspect_package_capabilities(
        &self,
        params: Parameters<AuditInput>,
    ) -> Result<String, String> {
        let dir = match &params.0.path {
            Some(p) => std::path::PathBuf::from(p),
            None => std::env::current_dir().map_err(|e| format!("cannot read cwd: {e}"))?,
        };
        match crate::audit::audit_path(&dir) {
            Ok(report) => Ok(crate::audit::report_json(&report)),
            Err(e) => Err(format!("{e}")),
        }
    }
}

fn resource_entries() -> Vec<Resource> {
    [
        (RESOURCE_GRAMMAR, "Rasmalai grammar", "Syntax, types, operators, and control-flow rules."),
        (RESOURCE_ARCHITECTURE, "Rasmalai architecture", "ARC memory model, GenRefs, and systems-language contrasts."),
        (RESOURCE_MANIFEST, "Project.config manifest", "Manifest layout, SemVer deps, and @std imports."),
        (RESOURCE_STDLIB_API, "Rasmalai stdlib API", "Complete @std/* symbol catalog as JSON."),
    ]
    .into_iter()
    .map(|(uri, name, description)| {
        Resource::new(uri, name)
            .with_description(description)
            .with_mime_type(if uri.ends_with(".json") {
                "application/json"
            } else {
                "text/markdown"
            })
    })
    .collect()
}

fn read_resource_text(uri: &str) -> Result<(String, String), McpError> {
    match uri {
        RESOURCE_GRAMMAR => Ok((GRAMMAR_SPEC.to_string(), "text/markdown".to_string())),
        RESOURCE_ARCHITECTURE => Ok((ARCHITECTURE_SPEC.to_string(), "text/markdown".to_string())),
        RESOURCE_MANIFEST => Ok((MANIFEST_SPEC.to_string(), "text/markdown".to_string())),
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
        _ => Err(McpError::resource_not_found(format!("unknown resource `{uri}`"), None)),
    }
}

fn expert_prompt_text() -> String {
    format!(
        "You are a Rasmalai expert. Core invariants: only `Int` and `Float` exist (no sized integers); \
`let` bindings are mutable by default (no `mut`); classes are built with `new` like `new Meter(20)` \
(a direct class call is E204) while structs and records use direct calls like `Point(1.0, 2.0)`; single-statement \
`if`/`else` bodies are valid but `let` inside them is E108; lone `;` is an \
empty statement; memory is deterministic scope-based ARC and cycles must use `GenRef` back-edges.\n\n\
Read these resources for full context: `{RESOURCE_GRAMMAR}` for syntax and `{RESOURCE_ARCHITECTURE}` \
for the memory model. Always run the `check` tool on generated code before answering, and use \
`rasmalai_lookup_symbol` to confirm `@std/*` signatures instead of guessing them."
    )
}

fn convert_prompt_text(source_language: &str, code: &str) -> String {
    format!(
        "Translate the following {source_language} code into idiomatic Rasmalai.\n\n\
Rosetta rules: map dynamic objects to Rasmalai records or structs; erase Rust lifetimes into \
lexical ARC ownership (shared back-edges become `GenRef`); replace Go channels and goroutines with \
`@std/sync` threadpools, mutexes, or barriers; replace sized integers with `Int`/`Float`; map \
constructors to `new Class(...)` for classes and direct calls for structs and records; keep `fn Main(): Int` as the entry point when producing a \
runnable program. Verify the result with the `check` tool before answering.\n\n\
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
