use anyhow::Result;
use clap::Parser;
use sw_impact::cli::Cli;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    sw_impact::init_tracing(cli.verbose);
    sw_impact::run(cli)
}
