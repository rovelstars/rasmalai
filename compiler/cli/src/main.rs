#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

mod cmd;
use clap::{CommandFactory, FromArgMatches};

fn main() {
    let raw: Vec<String> = std::env::args().collect();
    let color_mode = cli::term::detect_color_mode(&raw);
    diagnostics::theme::set_color_mode(color_mode);
    let no_color_flag = raw.iter().any(|a| a == "--no-color");
    if no_color_flag || color_mode == diagnostics::theme::ColorMode::Never {
        diagnostics::theme::force_plain_output();
    }
    let plain = no_color_flag
        || color_mode == diagnostics::theme::ColorMode::Never
        || matches!(color_mode, diagnostics::theme::ColorMode::Auto)
            && (std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty())
                || std::env::var("TERM").map(|t| t == "dumb").unwrap_or(false)
                || std::env::var("CLICOLOR").map(|v| v == "0").unwrap_or(false));
    let cmd = cli::args::Cli::command()
        .long_version(Box::leak(cli::version::long_version().into_boxed_str()) as &str)
        .color(match color_mode {
            diagnostics::theme::ColorMode::Always => clap::ColorChoice::Always,
            diagnostics::theme::ColorMode::Never => clap::ColorChoice::Never,
            diagnostics::theme::ColorMode::Auto if plain => clap::ColorChoice::Never,
            diagnostics::theme::ColorMode::Auto => clap::ColorChoice::Auto,
        });
    let matches = cmd.try_get_matches_from(raw).unwrap_or_else(|e| e.exit());
    let args = cli::args::Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());
    let color_mode = match (args.no_color, args.color) {
        (true, _) => diagnostics::theme::ColorMode::Never,
        (false, cli::args::ColorWhen::Auto) => color_mode,
        (false, cli::args::ColorWhen::Always) => diagnostics::theme::ColorMode::Always,
        (false, cli::args::ColorWhen::Never) => diagnostics::theme::ColorMode::Never,
    };
    diagnostics::theme::set_color_mode(color_mode);
    if color_mode == diagnostics::theme::ColorMode::Never {
        diagnostics::theme::force_plain_output();
    }
    if args.verbose {
        eprintln!("rnx: verbose logging enabled");
    }
    let sysroot = sysroot::Sysroot::discover(args.sysroot.as_deref());
    if matches.value_source("sysroot") == Some(clap::parser::ValueSource::CommandLine) {
        if let Some(dir) = args.sysroot.as_deref() {
            unsafe { std::env::set_var(sysroot::ENV_VAR, dir) };
        }
    }
    if let Some(what) = args.print.as_deref() {
        if what == "sysroot" {
            if let Err(e) = sysroot.validate() {
                eprintln!("error: invalid sysroot: {e}");
                std::process::exit(1);
            }
            println!("{}", sysroot.display_root());
            return;
        }
        eprintln!("error: unsupported --print {what}; expected `sysroot`");
        std::process::exit(2);
    }
    if !sysroot.is_host() {
        if let Err(e) = sysroot.validate() {
            eprintln!("error: invalid sysroot: {e}");
            std::process::exit(1);
        }
    }
    if args.offline {
        unsafe { std::env::set_var("RNX_OFFLINE", "1") };
    }
    let quiet = args.quiet;
    let verbose = args.verbose;
    let no_color = args.no_color;
    let command = match args.command {
        Some(c) => c,
        None => {
            if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                cli::args::Command::Repl
            } else {
                eprintln!("No subcommand given. Run `rnx --help` for the command list, or `rnx repl` for a shell.");
                std::process::exit(2);
            }
        }
    };
    if let Some((root, requested)) = cmd::target_selection_input(&command)
        && let Err(e) = cmd::activate_target(&root, requested.as_deref())
    {
        eprintln!("{e}");
        std::process::exit(1);
    }
    cmd::fetch::install_registry_transport();
    let prefer = frontend::native::NativePrefer::resolve(args.prefer_system, args.prefer_bundled);
    cmd::dispatch(command, verbose, quiet, no_color, prefer);
}
