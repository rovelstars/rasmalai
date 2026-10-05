use clap::{Parser, Subcommand};
use clap_complete::Shell;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "rnx",
    author,
    version,
    about = "Rasmalai systems language compiler and developer toolchain",
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
        /// Fail unless Project.deplock matches the manifest
        #[arg(long)]
        locked: bool,
        /// Optimization level (0 or 1)
        #[arg(short = 'O', long = "opt-level", default_value = "1")]
        opt_level: String,
        /// Print per-pass compile timings to stderr
        #[arg(long)]
        time_passes: bool,
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
        /// Fail unless Project.deplock matches the manifest
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
        /// Write a Chrome trace event file
        #[arg(long)]
        trace: Option<PathBuf>,
        /// Append JIT symbol entries for perf tooling
        #[arg(long)]
        perf_map: bool,
        /// Accepted for parity with build; ignored for run
        #[arg(short = 'g', long = "debug")]
        debug: bool,
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
    /// Fetch git dependencies into the module cache
    Fetch {
        /// Package selection
        #[arg(short = 'p', long = "package")]
        package: Option<String>,
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
        /// Document the embedded standard library instead of a project
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
    /// Publish a package archive to the Rasmalai registry
    Publish {
        /// Path to a pre-built .tar.gz archive (otherwise packs the current project)
        tarball: Option<PathBuf>,
        /// Registry endpoint URL
        #[arg(long, default_value = "https://rnx.dev/api/packages")]
        registry: String,
        /// Publisher authentication token (or RNX_TOKEN)
        #[arg(long, env = "RNX_TOKEN")]
        token: Option<String>,
    },
    /// Vendor git dependencies into vendor/
    Vendor {
        /// Package selection
        #[arg(short = 'p', long = "package")]
        package: Option<String>,
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
    },
    /// Add a local package dependency with capability approval
    Add {
        /// Dependency name (must match its `Project.config` name)
        package: String,
        /// Local directory containing the package
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
    },
    /// Inspect and prune the build cache
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
    /// Prune least-recently-used global cache entries over the byte cap
    Prune {
        /// Keep at most this many bytes (default 2 GiB)
        #[arg(long)]
        max_bytes: Option<u64>,
    },
    /// Show global and project cache usage
    Status,
}
