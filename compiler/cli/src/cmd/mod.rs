fn parse_opt_level(s: &str) -> u8 {
    match s {
        "0" => 0,
        "1" => 1,
        _ => {
            eprintln!("opt level must be 0 or 1");
            std::process::exit(2);
        }
    }
}

fn open_browser(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let prog = "open";
    #[cfg(not(target_os = "macos"))]
    let prog = "xdg-open";
    std::process::Command::new(prog).arg(path).spawn().map(|_| ())
}

fn backend_name(backend: &cli::TestBackend) -> &'static str {
    match backend {
        cli::TestBackend::Interpreter => "interpreter",
        cli::TestBackend::Cranelift => "cranelift",
        cli::TestBackend::Llvm => "llvm",
    }
}

fn bus_name(stage: &str) -> &str {
    match stage {
        "lex_parse" => "parse",
        "typecheck" => "typecheck",
        "lower" => "ir-lower",
        "opt_pipeline" => "opt",
        "verify" => "verify",
        "codegen" => "codegen",
        other => other,
    }
}

fn bus_detail(stage: &str) -> &str {
    match stage {
        "lex_parse" => "lex & parse sources",
        "typecheck" => "soundness & borrow analysis",
        "lower" => "lir lowering",
        "opt_pipeline" => "optimization pipeline",
        "verify" => "ir verification",
        "codegen" => "native object emission",
        _ => "",
    }
}

mod add;
mod audit;
mod bench;
mod build;
mod cache;
mod check;
mod completions;
mod dev;
mod doc;
mod explain;
mod fetch;
mod fmt;
mod init;
mod lint;
mod lock;
mod lsp;
mod mcp;
mod pack;
mod publish;
mod repl;
mod run;
mod setup;
mod test;
mod unpack;
mod vendor;

use self::add::run_add;
use self::audit::run_audit;
use self::bench::run_bench;
use self::build::run_build;
use self::cache::{run_cache, run_clean};
use self::check::run_check;
use self::completions::run_completions;
use self::dev::run_dev;
use self::doc::run_doc;
use self::explain::run_explain;
use self::fetch::run_fetch;
use self::fmt::run_fmt;
use self::init::run_init;
use self::lint::run_lint;
use self::lock::run_lock;
use self::lsp::run_lsp;
use self::mcp::run_mcp;
use self::pack::run_pack;
use self::publish::run_publish;
use self::repl::run_repl;
use self::run::run_run;
use self::setup::run_setup;
use self::test::run_test;
use self::unpack::run_unpack;
use self::vendor::run_vendor;

pub(super) fn dispatch(command: cli::args::Command, verbose: bool, quiet: bool, no_color: bool) {
    match command {
            cli::args::Command::Repl => {
                run_repl();
            }
            cli::args::Command::Dev { path, entry, no_rerun, mcp } => {
                run_dev(path, entry, no_rerun, mcp);
            }
            cli::args::Command::Check {
                paths,
                package,
                json,
            } => {
                run_check(paths, package, json, verbose, quiet);
            }
            cli::args::Command::Run {
                path,
                entry,
                package,
                locked,
                opt_level,
                backend,
                time_passes,
                trace,
                perf_map,
                debug,
                args: rest,
            } => {
                run_run(path, entry, package, locked, opt_level, backend, time_passes, trace, perf_map, debug, rest);
            }
            cli::args::Command::Explain { code } => {
                run_explain(code);
            }
            cli::args::Command::Build {
                path,
                entry,
                out,
                release,
                lib,
                emit_obj,
                target: target_triple,
                locked,
                opt_level,
                time_passes,
                trace,
                perf_map,
                debug,
                package,
            } => {
                run_build(path, entry, out, release, lib, emit_obj, target_triple, locked, opt_level, time_passes, trace, perf_map, debug, package, verbose, quiet);
            }
            cli::args::Command::Init { name } => {
                run_init(name);
            }
            cli::args::Command::Lsp => {
                run_lsp();
            }
            cli::args::Command::Lint {
                paths,
                package,
                sarif,
                json,
                deny_warnings,
                fix,
            } => {
                run_lint(paths, package, sarif, json, deny_warnings, fix);
            }
            cli::args::Command::Fmt { paths, check, diff } => {
                run_fmt(paths, check, diff);
            }
            cli::args::Command::Lock { package } => {
                run_lock(package);
            }
            cli::args::Command::Fetch { package } => {
                run_fetch(package);
            }
            cli::args::Command::Doc {
                package,
                open,
                no_deps,
                all,
                json,
                out_dir,
                stdlib,
            } => {
                run_doc(package, open, no_deps, all, json, out_dir, stdlib, quiet);
            }
            cli::args::Command::Pack {
                package,
                out_dir,
                gzip,
            } => {
                run_pack(package, out_dir, gzip);
            }
            cli::args::Command::Publish {
                tarball,
                registry,
                token,
            } => {
                run_publish(tarball, registry, token);
            }
            cli::args::Command::Unpack { archive, out_dir } => {
                run_unpack(archive, out_dir);
            }
            cli::args::Command::Vendor { package } => {
                run_vendor(package);
            }
            cli::args::Command::Audit {
                path,
                json,
                export_manifest,
            } => {
                run_audit(path, json, export_manifest);
            }
            cli::args::Command::Add { package, path, accept_caps, accept_all_caps } => {
                run_add(package, path, accept_caps, accept_all_caps);
            }
            cli::args::Command::Bench {
                package,
                filter,
                backend,
                release: _,
                no_release,
                time_passes,
                trace,
                perf_map,
                debug,
            } => {
                run_bench(package, filter, backend, no_release, time_passes, trace, perf_map, debug);
            }
            cli::args::Command::Test {
                filter,
                package,
                opt_level,
                backend,
                exact,
                nocapture,
            } => {
                run_test(filter, package, opt_level, backend, exact, nocapture, verbose, quiet, no_color);
            }
            cli::args::Command::Setup { editor } => {
                run_setup(editor);
            }
            cli::args::Command::Completions { shell } => {
                run_completions(shell);
            }
            cli::args::Command::Mcp => {
                run_mcp();
            }
            cli::args::Command::Cache { action } => {
                run_cache(action);
            }
            cli::args::Command::Clean { package } => {
                run_clean(package);
            }
    }
}
