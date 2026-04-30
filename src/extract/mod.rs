pub mod admin;
pub mod config;
pub mod php;
pub mod twig;

use std::path::Path;

use anyhow::Result;

use crate::model::{Fact, FactRole};
use crate::walker::Language;

pub fn extract_facts(
    language: Language,
    content: &str,
    relative_path: &Path,
    role: FactRole,
    include_snippets: bool,
) -> Result<Vec<Fact>> {
    match language {
        Language::Php => php::extract(content, relative_path, role, include_snippets),
        Language::Twig => Ok(twig::extract(
            content,
            relative_path,
            role,
            include_snippets,
        )),
        Language::Xml => config::extract_xml(content, relative_path, role, include_snippets),
        Language::Json => config::extract_json(content, relative_path, role, include_snippets),
        Language::Yaml => config::extract_yaml(content, relative_path, role, include_snippets),
        Language::Toml => config::extract_toml(content, relative_path, role, include_snippets),
        Language::JavaScript | Language::TypeScript | Language::Vue => Ok(admin::extract(
            content,
            relative_path,
            role,
            include_snippets,
        )),
        Language::Unknown => Ok(Vec::new()),
    }
}
