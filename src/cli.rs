use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "sw-impact")]
#[command(about = "Report third-party Shopware extensions touching changed Shopware surfaces")]
pub struct Cli {
    #[arg(short, long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Build a plugin corpus index.
    Index(IndexArgs),
    /// Check a local Shopware worktree against an index.
    Check(CheckArgs),
    /// Query surface keys or wildcard patterns.
    Query(QueryArgs),
}

#[derive(Debug, Args, Clone)]
pub struct IndexArgs {
    #[arg(long)]
    pub plugins: PathBuf,

    #[arg(long)]
    pub out: PathBuf,

    #[arg(long, default_value = "auto")]
    pub threads: String,

    #[arg(long)]
    pub force: bool,

    #[arg(long)]
    pub no_snippets: bool,

    #[arg(long)]
    pub include_low_confidence: bool,

    #[arg(long, default_value = "**/composer.json")]
    pub plugin_manifest: String,

    #[arg(
        long,
        value_delimiter = ',',
        default_value = "vendor,node_modules,dist,build,var,cache"
    )]
    pub exclude: Vec<String>,

    #[arg(long, default_value = "2MiB")]
    pub max_file_size: String,
}

#[derive(Debug, Args, Clone)]
pub struct CheckArgs {
    #[arg(long)]
    pub shopware: PathBuf,

    #[arg(long, default_value = "origin/trunk")]
    pub base: String,

    #[arg(long)]
    pub index: PathBuf,

    #[arg(long)]
    pub include_low_confidence: bool,

    #[arg(long)]
    pub only_high_confidence: bool,

    #[arg(long, default_value_t = 20)]
    pub max_evidence_per_surface: usize,
}

#[derive(Debug, Args, Clone)]
pub struct QueryArgs {
    #[arg(long)]
    pub index: PathBuf,

    #[arg(long)]
    pub include_low_confidence: bool,

    #[arg(long)]
    pub only_high_confidence: bool,

    #[arg(long, default_value_t = 50)]
    pub max_evidence: usize,

    pub pattern: String,
}
