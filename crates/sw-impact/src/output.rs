use std::fmt::Display;

use clap::ValueEnum;

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
