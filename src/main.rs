use clap::Parser;
use std::process::ExitCode;
use to_pdf::{cli::Cli, pipeline};

fn main() -> ExitCode {
    let cli = Cli::parse();
    let summary = pipeline::run(&cli);
    summary.report(cli.quiet, cli.verbose);
    if summary.failed.is_empty() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
