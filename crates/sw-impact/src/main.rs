use std::{path::PathBuf, process::ExitCode};

use clap::{Args, Parser, Subcommand};
use sw_impact_scanner::{
    api::{load_surface_map, save_surface_map},
    report::Report,
    scan_dir, validate_queries,
};
use tracing_subscriber::{EnvFilter, FmtSubscriber};

use crate::output::{OutputFormat, ReportDiagnosticExt};

mod output;

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
    /// Index shopware codebase for public API
    Index(IndexArgs),
    /// Check shopware codebase against index for breaking changes
    Check(CheckArgs),
}

#[derive(Debug, Args, Clone)]
pub struct IndexArgs {
    /// Path to shopware codebase root folder
    pub path: PathBuf,
    #[arg(short, long, default_value = "./index.json")]
    pub index: PathBuf,
}

#[derive(Debug, Args, Clone)]
pub struct CheckArgs {
    /// Path to shopware codebase root folder
    pub path: PathBuf,
    #[arg(short, long, default_value = "./index.json")]
    pub index: PathBuf,
    /// Which output format you want to have
    #[arg(short, long, default_value_t = OutputFormat::Human)]
    pub format: OutputFormat,
}

fn main() -> ExitCode {
    setup_tracing_subscriber();

    let cli = Cli::parse();
    validate_queries();

    match cli.command {
        Commands::Index(args) => {
            let surface_index = scan_dir(&args.path).unwrap();
            if surface_index.is_empty() {
                eprintln!("no public api surfaces found");
                return ExitCode::FAILURE;
            }

            save_surface_map(&surface_index, &args.index).unwrap();
            eprintln!("saved index with {} api surfaces", surface_index.len());
            ExitCode::SUCCESS
        }
        Commands::Check(args) => {
            let surface_index = load_surface_map(&args.index).unwrap();
            eprintln!("loaded index with {} api surfaces", surface_index.len());
            let new_surface = scan_dir(&args.path).unwrap();

            let report = Report::build_report(&surface_index, &new_surface);

            if !report.contains_breaks() {
                eprintln!("no public api breaking changes found");
                return ExitCode::SUCCESS;
            }

            report.output_diagnostics(args.format);
            ExitCode::FAILURE
        }
    }
}

/// setup tracing logging, goes to stderr to keep the normal stdout output clean
/// only outputs warn or errors by default, but can be changed via RUST_LOG=trace env variable
fn setup_tracing_subscriber() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    let subscriber = FmtSubscriber::builder()
        .with_env_filter(filter)
        .with_ansi(true)
        .with_level(true)
        .with_thread_ids(true)
        .with_writer(std::io::stderr)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .expect("setting default tracing subscriber failed");
}
