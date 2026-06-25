use std::{fmt::Display, path::Path};

use clap::ValueEnum;
use sw_impact_scanner::report::Report;

mod github;
mod human;
mod json;

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum OutputFormat {
    /// Pretty text to stdout
    Human,
    /// Workflow annotations to stdout
    Github,
    /// Json to stdout
    Json,
}

impl Display for OutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutputFormat::Human => write!(f, "human"),
            OutputFormat::Github => write!(f, "github"),
            OutputFormat::Json => write!(f, "json"),
        }
    }
}

pub trait ReportDiagnosticExt {
    fn output_diagnostics(&self, format: OutputFormat, source_root: &Path);
}

impl ReportDiagnosticExt for Report {
    fn output_diagnostics(&self, format: OutputFormat, source_root: &Path) {
        match format {
            OutputFormat::Human => human::output_human(self, source_root),
            OutputFormat::Github => github::output_github(self),
            OutputFormat::Json => json::output_json(self),
        }
    }
}
