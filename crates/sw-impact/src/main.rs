use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use sw_impact_scanner::{
    api::{load_surface_map, save_surface_map},
    scan_dir, validate_queries,
};
use tracing_subscriber::{EnvFilter, FmtSubscriber};

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
    validate_queries();

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    let subscriber = FmtSubscriber::builder()
        .with_env_filter(filter)
        .with_ansi(true)
        .with_level(true)
        .with_thread_ids(true)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .expect("setting default tracing subscriber failed");

    match cli.command {
        Commands::Scan(args) => {
            let surfaces = scan_dir(&args.path).unwrap();
            // TODO: clean up persist surfaces
            save_surface_map(&surfaces, "./index.json").unwrap();
            let loaded = load_surface_map("./index.json").unwrap();
            println!("detected {} api surfaces", loaded.len());
        }
    }
}
