mod cmd;
use clap::{CommandFactory, FromArgMatches};

fn main() {
    let raw: Vec<String> = std::env::args().collect();
    let plain = std::env::var_os("NO_COLOR").is_some()
        || raw.iter().any(|a| a == "--no-color");
    if raw.iter().any(|a| a == "--no-color") {
        diagnostics::theme::force_plain_output();
    }
    let cmd = cli::args::Cli::command()
        .color(if plain {
            clap::ColorChoice::Never
        } else {
            clap::ColorChoice::Auto
        });
    let matches = cmd.try_get_matches_from(raw).unwrap_or_else(|e| e.exit());
    let args = cli::args::Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());    if args.verbose {
        eprintln!("rnx: verbose logging enabled");
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
                eprintln!("rnx: missing subcommand (try `rnx repl`)");
                std::process::exit(2);
            }
        }
    };
    cmd::dispatch(command, verbose, quiet, no_color);
}
