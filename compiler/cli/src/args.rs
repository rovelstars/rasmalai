use clap::{Parser, Subcommand};
use clap_complete::Shell;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "rnx",
    author,
    version,
    about = "Rasmalai systems language compiler and developer toolchain",
    long_about = "Rasmalai systems language compiler and developer toolchain.\n\n`rnx --version` prints the full build record: version, commit hash, build timestamp, install kind, target triple, and backend versions.",
    styles = crate::styles::aura_clap_styles()
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Strip all ANSI color escapes from terminal output
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Enable verbose diagnostic and pipeline logging
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Suppress progress banners and non-error telemetry
    #[arg(short, long, global = true)]
    pub quiet: bool,

    /// Resolve every native dependency from the host system (`pkg-config` verified, fail-closed)
    #[arg(long, global = true, conflicts_with = "prefer_bundled")]
    pub prefer_system: bool,

    /// Resolve every native dependency from bundled OCI blobs (role-plus-platform match)
    #[arg(long, global = true, conflicts_with = "prefer_system")]
    pub prefer_bundled: bool,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Start an interactive JIT shell
    Repl,
    /// Run with hot reload: watch sources, swap function bodies without restart
    Dev {
        /// Target entry file (defaults to current directory)
        path: Option<PathBuf>,
        /// Entry function name
        #[arg(long, default_value = "Main")]
        entry: String,
        /// Never re-invoke the entry after a swap (for background services)
        #[arg(long = "no-rerun")]
        no_rerun: bool,
        /// Serve MCP over stdio alongside the watcher (eval_code, hot_reload, get_diagnostics)
        #[arg(long)]
        mcp: bool,
    },
    /// Compile package or file into a native binary
    Build {
        /// Target entry path or directory (defaults to current directory)
        path: Option<PathBuf>,
        /// Entry function name
        #[arg(long, default_value = "Main")]
        entry: String,
        /// Build with optimizations enabled
        #[arg(long)]
        release: bool,
        /// Build a static library instead of an executable
        #[arg(long)]
        lib: bool,
        /// Emit an unlinked object file and skip linking
        #[arg(long)]
        emit_obj: bool,
        /// Target triple override (e.g. x86_64-unknown-linux-gnu)
        #[arg(long)]
        target: Option<String>,
        /// Verify Project.deplock checksums (floating ranges always resolve from the lock; accepted for compatibility)
        #[arg(long)]
        locked: bool,
        /// Optimization level (0 or 1)
        #[arg(short = 'O', long = "opt-level", default_value = "1")]
        opt_level: String,
        /// Print per-pass compile timings to stderr
        #[arg(long)]
        time_passes: bool,
        /// Dump macro expansions before/after to `.rnx-cache/expansions/`
        #[arg(long)]
        expand_macros: bool,
        /// Write a Chrome trace event file
        #[arg(long)]
        trace: Option<PathBuf>,
        /// Append JIT symbol entries for perf tooling
        #[arg(long)]
        perf_map: bool,
        /// Emit native debug info (applies to build outputs)
        #[arg(short = 'g', long = "debug")]
        debug: bool,
        /// Package selection for workspace builds
        #[arg(short = 'p', long = "package")]
        package: Option<String>,
        /// Parallel worker count (or RNX_JOBS). Default: available parallelism
        #[arg(long, env = "RNX_JOBS")]
        jobs: Option<usize>,
        /// Build memory cap in bytes; KB/MB/GB suffixes allowed (or RNX_MEMORY_CAP). Default: min(50% RAM, 8GB); 4GB when RAM size is unreadable
        #[arg(long, env = "RNX_MEMORY_CAP", value_parser = crate::parse_memory_cap)]
        memory_cap: Option<u64>,
        /// Promote performance warnings to hard errors: bare flag denies all (P201-P203); or comma list, repeatable, e.g. --deny-perf-lints=P201,P203 (or RNX_DENY_PERF_LINTS when the flag is absent; the flag overrides the env)
        #[arg(long = "deny-perf-lints", num_args = 0..=1, default_missing_value = "ALL", require_equals = true, value_delimiter = ',')]
        deny_perf_lints: Vec<String>,
        /// Show Tier-1 performance notes (P101/P102); hidden by default (or RNX_SHOW_PERF_NOTES=1 when the flag is absent; the flag overrides the env)
        #[arg(long = "show-perf-notes")]
        show_perf_notes: bool,
        /// Apply the P201 ZeroInit autofix: preview a unified diff, rewrite sources in place with .bak backups, then verify (re-parse, rebuild, receipt, checksum)
        #[arg(long = "fix")]
        fix: bool,
        /// Overwrite existing .bak backups when fixing
        #[arg(long = "fix-force")]
        fix_force: bool,
        /// Keep .bak backups after a successful fix
        #[arg(long = "fix-keep-backup")]
        fix_keep_backup: bool,
    },
    /// Fast type-check and semantic validation
    Check {
        /// Files or directories to check
        paths: Vec<PathBuf>,
        /// Package selection for workspace checks
        #[arg(short = 'p', long = "package")]
        package: Option<String>,
        /// Emit machine-readable JSON diagnostics
        #[arg(long)]
        json: bool,
        /// Dump macro expansions before/after to `.rnx-cache/expansions/`
        #[arg(long)]
        expand_macros: bool,
    },
    /// Compile and execute the immediate entry module
    Run {
        /// Target entry path or directory (defaults to current directory)
        path: Option<PathBuf>,
        /// Entry function name
        #[arg(long, default_value = "Main")]
        entry: String,
        /// Package selection
        #[arg(short = 'p', long = "package")]
        package: Option<String>,
        /// Verify Project.deplock checksums (floating ranges always resolve from the lock; accepted for compatibility)
        #[arg(long)]
        locked: bool,
        /// Optimization level (0 or 1)
        #[arg(short = 'O', long = "opt-level", default_value = "1")]
        opt_level: String,
        /// Execution backend: cranelift (default), interpreter, or llvm
        #[arg(long, default_value = "cranelift")]
        backend: String,
        /// Print per-pass compile timings to stderr
        #[arg(long)]
        time_passes: bool,
        /// Dump macro expansions before/after to `.rnx-cache/expansions/`
        #[arg(long)]
        expand_macros: bool,
        /// Write a Chrome trace event file
        #[arg(long)]
        trace: Option<PathBuf>,
        /// Append JIT symbol entries for perf tooling
        #[arg(long)]
        perf_map: bool,
        /// Accepted for parity with build; ignored for run
        #[arg(short = 'g', long = "debug")]
        debug: bool,
        /// Parallel worker count (or RNX_JOBS). Default: available parallelism
        #[arg(long, env = "RNX_JOBS")]
        jobs: Option<usize>,
        /// Build memory cap in bytes; KB/MB/GB suffixes allowed (or RNX_MEMORY_CAP). Default: min(50% RAM, 8GB); 4GB when RAM size is unreadable
        #[arg(long, env = "RNX_MEMORY_CAP", value_parser = crate::parse_memory_cap)]
        memory_cap: Option<u64>,
        /// Arguments passed directly to the compiled executable
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Print the meaning and fix hint for a diagnostic code
    Explain {
        /// Diagnostic code to explain
        code: Option<String>,
    },
    /// Scaffold a new Rasmalai project
    Init {
        /// Project name
        name: Option<String>,
    },
    /// Configure editor integration (LSP and highlighting)
    Setup {
        /// Editor to configure: vscode, zed, helix, neovim
        editor: Option<String>,
    },
    /// Spawn the language server over standard I/O
    Lsp,
    /// Analyze source code for diagnostics and anti-patterns
    Lint {
        /// Files or directories to lint
        paths: Vec<PathBuf>,
        /// Package selection
        #[arg(short = 'p', long = "package")]
        package: Option<String>,
        /// Emit SARIF report
        #[arg(long)]
        sarif: bool,
        /// Emit machine-readable JSON diagnostics
        #[arg(long)]
        json: bool,
        /// Fail when any warning is present
        #[arg(long)]
        deny_warnings: bool,
        /// Apply safe mechanical fixes (underscore renames)
        #[arg(long)]
        fix: bool,
    },
    /// Format source files according to the style specification
    Fmt {
        /// Files or directories to format (defaults to current directory)
        paths: Vec<PathBuf>,
        /// Report unformatted files without writing
        #[arg(long)]
        check: bool,
        /// Print unified diffs without writing
        #[arg(long)]
        diff: bool,
    },
    /// Resolve dependencies and write Project.deplock
    Lock {
        /// Package selection
        #[arg(short = 'p', long = "package")]
        package: Option<String>,
    },
    /// Re-resolve registry ranges and rewrite Project.deplock with new digests
    Update {
        /// Only update these dependencies (`name` or `name@range`; `latest` resolves then pins)
        packages: Vec<String>,
        /// Allow crossing major versions (explicit `name@range` selectors always may)
        #[arg(long)]
        major: bool,
        /// Package selection for workspace builds
        #[arg(short = 'p', long = "package")]
        package: Option<String>,
    },
    /// Fetch dependencies into the cache and precompile dep products
    Fetch {
        /// Package selection
        #[arg(short = 'p', long = "package")]
        package: Option<String>,
    },
    /// Download libraries from the registry into the cache
    Download {
        /// Packages to download (`name` or `name@range`; seeds every `@std/*` when empty)
        packages: Vec<String>,
        /// Seed the project-local cache instead of the global cache
        #[arg(long)]
        local: bool,
        /// Registry endpoint URL (or RNX_REGISTRY)
        #[arg(long, env = "RNX_REGISTRY")]
        registry: Option<String>,
    },
    /// Check toolchain health and optionally repair the stdlib cache
    Doctor {
        /// Re-download missing standard library packages into the global cache
        #[arg(long = "repair-std")]
        repair_std: bool,
        /// Registry endpoint URL (or RNX_REGISTRY)
        #[arg(long, env = "RNX_REGISTRY")]
        registry: Option<String>,
    },
    /// Generate static documentation site
    Doc {
        /// Package selection
        #[arg(short = 'p', long = "package")]
        package: Option<String>,
        /// Open the rendered site in a browser
        #[arg(long)]
        open: bool,
        /// Skip path-dependency packages
        #[arg(long)]
        no_deps: bool,
        /// Include default and private items, not just `pub`
        #[arg(long, visible_alias = "private")]
        all: bool,
        /// Emit machine-readable JSON instead of the HTML site
        #[arg(long)]
        json: bool,
        /// Write output under this directory instead of `target/doc`
        #[arg(long)]
        out_dir: Option<PathBuf>,
        /// Document the standard library (from the seeded global cache) instead of a project
        #[arg(long)]
        stdlib: bool,
    },
    /// Pack the project into a distributable archive
    Pack {
        /// Package selection
        #[arg(short = 'p', long = "package")]
        package: Option<String>,
        /// Output directory for archives
        #[arg(long)]
        out_dir: Option<PathBuf>,
        /// Compress archives with gzip
        #[arg(long, short = 'z')]
        gzip: bool,
    },
    /// Unpack a distributable archive
    Unpack {
        /// Archive file to unpack
        archive: Option<String>,
        /// Destination directory
        #[arg(long)]
        out_dir: Option<String>,
    },
    /// Publish the current project to the Rasmalai registry
    Publish {
        /// Registry base URL
        #[arg(long, default_value = "https://rasmalai.rovelstars.com")]
        registry: String,
        /// Publisher authentication token (or RNX_TOKEN)
        #[arg(long, env = "RNX_TOKEN")]
        token: Option<String>,
        /// Author-asserted OCI index digest for the `artifacts` image (one digest for the one image)
        #[arg(long)]
        artifact: Option<String>,
    },
    /// Remove a version published in the last 72 hours (`name@version`)
    Unpublish {
        /// Version coordinate to remove
        target: String,
        /// Registry base URL
        #[arg(long, default_value = "https://rasmalai.rovelstars.com")]
        registry: String,
        /// Publisher authentication token (or RNX_TOKEN)
        #[arg(long, env = "RNX_TOKEN")]
        token: Option<String>,
    },
    /// Copy local package sources into .vendor/ (downloads from the registry first when missing)
    Vendor {
        /// Packages to vendor (defaults to the scope's git dependencies)
        packages: Vec<String>,
        /// Destination directory (defaults to .vendor/ under the scope root)
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Audit a package directory for security capabilities
    Audit {
        /// Package directory to audit (defaults to current directory)
        #[arg(long)]
        path: Option<PathBuf>,
        /// Emit the full capability report as JSON
        #[arg(long)]
        json: bool,
        /// Emit the minimal publishing manifest as JSON
        #[arg(long)]
        export_manifest: bool,
        /// Report the resolved license graph (per-package SPDX, linkage, obligations, conflicts)
        #[arg(long)]
        licenses: bool,
        /// Fail when a locked dependency carries one of these SPDX ids (comma-separated, repeatable)
        #[arg(long = "deny-license", value_delimiter = ',')]
        deny_license: Vec<String>,
        /// Registry base URL for the advisory feed (or RNX_REGISTRY)
        #[arg(long, env = "RNX_REGISTRY")]
        registry: Option<String>,
    },
    /// Add a package dependency with capability approval (`name[@range]` from the registry or `--path <dir>` local)
    Add {
        /// Dependency name (must match its `Project.config` name; append `@range` for registry versions)
        package: String,
        /// Local directory containing the package (omit to resolve `package` from the registry)
        #[arg(long)]
        path: Option<PathBuf>,
        /// Approve exactly these comma-separated capabilities non-interactively
        #[arg(long)]
        accept_caps: Option<String>,
        /// Approve all deduced capabilities non-interactively
        #[arg(long)]
        accept_all_caps: bool,
    },
    /// Run benchmark blocks and report timings
    Bench {
        /// Package selection
        #[arg(short = 'p', long = "package")]
        package: Option<String>,
        /// Run only benchmarks matching this substring
        #[arg(long)]
        filter: Option<String>,
        /// Execution backend: llvm (default), interpreter, or cranelift
        #[arg(long, default_value = "llvm")]
        backend: String,
        /// Build with optimizations enabled
        #[arg(long)]
        release: bool,
        /// Build without optimizations
        #[arg(long)]
        no_release: bool,
        /// Print per-pass compile timings to stderr
        #[arg(long)]
        time_passes: bool,
        /// Write a Chrome trace event file
        #[arg(long)]
        trace: Option<PathBuf>,
        /// Append JIT symbol entries for perf tooling
        #[arg(long)]
        perf_map: bool,
        /// Accepted for parity with build; ignored for bench
        #[arg(short = 'g', long = "debug")]
        debug: bool,
        /// Parallel worker count (or RNX_JOBS). Default: available parallelism
        #[arg(long, env = "RNX_JOBS")]
        jobs: Option<usize>,
        /// Build memory cap in bytes; KB/MB/GB suffixes allowed (or RNX_MEMORY_CAP). Default: min(50% RAM, 8GB); 4GB when RAM size is unreadable
        #[arg(long, env = "RNX_MEMORY_CAP", value_parser = crate::parse_memory_cap)]
        memory_cap: Option<u64>,
    },
    /// Run package unit tests and integration suites
    Test {
        /// Filter test names containing this substring
        filter: Option<String>,
        /// Package selection
        #[arg(short = 'p', long = "package")]
        package: Option<String>,
        /// Optimization level (0 or 1)
        #[arg(short = 'O', long = "opt-level", default_value = "1")]
        opt_level: String,
        /// Execution backend: cranelift (default), interpreter, or llvm
        #[arg(long, default_value = "cranelift")]
        backend: String,
        /// Match the filter against the full test name instead of by substring
        #[arg(long)]
        exact: bool,
        /// Do not capture stdout/stderr; print test output immediately
        #[arg(long)]
        nocapture: bool,
        /// Parallel worker count (or RNX_JOBS). Default: available parallelism
        #[arg(long, env = "RNX_JOBS")]
        jobs: Option<usize>,
        /// Build memory cap in bytes; KB/MB/GB suffixes allowed (or RNX_MEMORY_CAP). Default: min(50% RAM, 8GB); 4GB when RAM size is unreadable
        #[arg(long, env = "RNX_MEMORY_CAP", value_parser = crate::parse_memory_cap)]
        memory_cap: Option<u64>,
    },
    /// Inspect and prune the build cache (global content-addressed store plus project pools)
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },
    /// Remove project build outputs and the project cache
    Clean {
        /// Package selection
        #[arg(short = 'p', long = "package")]
        package: Option<String>,
    },
    /// Generate Rasmalai bindings from C headers
    Bindgen {
        /// Primary C headers to emit bindings from (repeatable)
        #[arg(long)]
        header: Vec<PathBuf>,
        /// Native library name for the generated `from native` import
        #[arg(long)]
        native_lib: String,
        /// Package name (defaults the upstream annotation name)
        #[arg(long)]
        pkg_name: String,
        /// Output directory for `bindgen.rnx` and `bindgen.oci.json`
        #[arg(long)]
        out: PathBuf,
        /// Target triple for preprocessing (default x86_64-unknown-linux-gnu)
        #[arg(long, default_value = "x86_64-unknown-linux-gnu")]
        target: String,
        /// Only emit symbols with one of these prefixes (empty means all)
        #[arg(long)]
        prefix: Vec<String>,
        /// Skip these exact symbol names
        #[arg(long)]
        skip: Vec<String>,
        /// Extra include directories for quoted headers
        #[arg(long)]
        include: Vec<PathBuf>,
        /// Case conversion for emitted wrapper and parameter names (keep, camel, pascal)
        #[arg(long, default_value = "keep")]
        case: String,
        /// Explicit rename of one C symbol (old=new, repeatable; old is the C name)
        #[arg(long)]
        rename: Vec<String>,
        /// Upstream name for the annotation set (defaults to the package name)
        #[arg(long)]
        upstream_name: Option<String>,
        /// Verify the output files are up to date without writing them
        #[arg(long)]
        check: bool,
    },
    /// Run an author-local task from Project.config (`tasks` table; never ships, never runs on consumers)
    Task {
        /// Task name (lists tasks when omitted; `bindgen` runs the `bindgen` driver table)
        name: Option<String>,
    },
    /// Generate shell completion scripts
    Completions {
        /// Target shell
        #[arg(value_enum)]
        shell: Shell,
    },
    /// Serve a Model Context Protocol (stdio) server exposing the toolchain
    Mcp,
}

/// Build cache management
#[derive(Subcommand, Debug, Clone)]
pub enum CacheAction {
    /// Prune least-recently-used global cache entries over the byte cap (also removes the old registry/ tree left by previous versions; the content-addressed store/ is retained)
    Prune {
        /// Keep at most this many bytes (default 2 GiB)
        #[arg(long)]
        max_bytes: Option<u64>,
    },
    /// Show global and project cache usage (global store/ plus project cache)
    Status,
}
