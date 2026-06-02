use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use sw_impact_scanner::scan_dir;

// Use MiMalloc, which is much more perfomant for small allocations
// on many platforms,
// matters in this CLI app because it processes lots of strings.
// Only downside is increased binary size.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[derive(Debug, Parser)]
pub struct Cli {
    #[arg(short, long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Scan shopware codebase for public API
    Scan(ScanArgs),
}

#[derive(Debug, Args, Clone)]
pub struct ScanArgs {
    pub path: PathBuf,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Scan(args) => scan_dir(&args.path).unwrap(),
    }
}
