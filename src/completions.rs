use clap::{Args, Command};
use clap_complete::{generate, Shell};

/// Generate shell completion scripts for bash, zsh, fish, or PowerShell.
#[derive(Args)]
pub struct CompletionsArgs {
    /// Shell to generate completions for
    #[arg(value_enum)]
    pub shell: Shell,
}

pub fn run(args: CompletionsArgs, cmd: &mut Command) {
    generate(args.shell, cmd, "remit", &mut std::io::stdout());
}
