pub mod check;
pub mod cli;
pub mod extract;
pub mod git;
pub mod model;
pub mod report;
pub mod source_link;
pub mod storage;
pub mod walker;

use anyhow::Result;
use cli::{Cli, Commands};

pub fn init_tracing(verbose: bool) {
    let filter = if verbose { "info" } else { "warn" };
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .without_time()
        .try_init();
}

pub fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Index(args) => storage::build_index(args, cli.verbose),
        Commands::Check(args) => check::run_check(args, cli.verbose),
        Commands::Query(args) => storage::query_index(args, cli.verbose),
    }
}
