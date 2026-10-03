use clap::CommandFactory;

pub(super) fn run_completions(shell: clap_complete::Shell) {
            let mut cmd = cli::args::Cli::command();
            let bin_name = cmd.get_name().to_string();
            clap_complete::generate(shell, &mut cmd, bin_name, &mut std::io::stdout());
}
