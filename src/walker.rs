use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use aho_corasick::AhoCorasick;
use anyhow::{Context, Result, anyhow};
use ignore::{DirEntry, WalkBuilder};
use rayon::prelude::*;
use serde_json::Value;

use crate::cli::IndexArgs;

const DEFAULT_EXCLUDES: &[&str] = &[
    "vendor",
    "node_modules",
    "dist",
    "build",
    "var",
    "cache",
    ".git",
];
const KNOWN_MANIFESTS: &[&str] = &["composer.json", "manifest.xml", "theme.json"];
const PREFILTER_ANCHORS: &[&str] = &[
    "Shopware\\",
    "Shopware.",
    "Shopware",
    "sw_extends",
    "@Storefront",
    "@Administration",
    "@Framework",
    "services.xml",
    "routes.xml",
    "composer.json",
    "manifest.xml",
    "repositoryFactory",
    "Component.override",
    "Component.extend",
    "Module.register",
    "PluginManager",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Php,
    Twig,
    Xml,
    Json,
    Yaml,
    Toml,
    JavaScript,
    TypeScript,
    Vue,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct PluginInfo {
    pub name: String,
    pub version: Option<String>,
    pub root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct CandidateFile {
    pub plugin: PluginInfo,
    pub absolute_path: PathBuf,
    pub relative_path: PathBuf,
    pub language: Language,
    pub size: u64,
}

#[derive(Debug, Default, Clone)]
pub struct WalkStats {
    pub plugins: usize,
    pub files_seen: usize,
    pub candidates: usize,
    pub skipped_large: usize,
    pub skipped_unsupported: usize,
}

pub fn discover_candidates(args: &IndexArgs) -> Result<(Vec<CandidateFile>, WalkStats)> {
    let plugins_root = args
        .plugins
        .canonicalize()
        .with_context(|| format!("plugins path does not exist: {}", args.plugins.display()))?;
    let excludes = excluded_components(&args.exclude);
    let max_file_size = parse_max_file_size(&args.max_file_size)?;
    let prefilter =
        AhoCorasick::new(PREFILTER_ANCHORS).context("failed to build corpus prefilter")?;

    let plugin_roots = discover_plugin_roots(&plugins_root, &excludes)?;
    let plugins = plugin_roots
        .par_iter()
        .map(|root| plugin_info_for_root(root))
        .collect::<Vec<_>>();

    let mut stats = WalkStats {
        plugins: plugins.len(),
        ..WalkStats::default()
    };
    let mut candidates = Vec::new();

    let plugin_results: Result<Vec<_>> = plugins
        .into_par_iter()
        .map(|plugin| {
            let mut plugin_candidates = Vec::new();
            let mut plugin_stats = WalkStats::default();

            collect_plugin_candidates(
                &plugin,
                &excludes,
                max_file_size,
                &prefilter,
                &mut plugin_candidates,
                &mut plugin_stats,
            )?;

            Ok((plugin_candidates, plugin_stats))
        })
        .collect();

    for (mut plugin_candidates, plugin_stats) in plugin_results? {
        stats.files_seen += plugin_stats.files_seen;
        stats.candidates += plugin_stats.candidates;
        stats.skipped_large += plugin_stats.skipped_large;
        stats.skipped_unsupported += plugin_stats.skipped_unsupported;
        candidates.append(&mut plugin_candidates);
    }

    candidates.sort_by(|left, right| {
        left.plugin
            .root
            .cmp(&right.plugin.root)
            .then_with(|| left.relative_path.cmp(&right.relative_path))
    });
    stats.candidates = candidates.len();

    Ok((candidates, stats))
}

pub fn language_for_path(path: &Path) -> Language {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let extension = path.extension().and_then(|extension| extension.to_str());

    if file_name.ends_with(".html.twig") || extension == Some("twig") {
        return Language::Twig;
    }

    match extension {
        Some("php") => Language::Php,
        Some("xml") => Language::Xml,
        Some("json") => Language::Json,
        Some("yaml") | Some("yml") => Language::Yaml,
        Some("toml") => Language::Toml,
        Some("js") | Some("mjs") | Some("cjs") => Language::JavaScript,
        Some("ts") => Language::TypeScript,
        Some("vue") => Language::Vue,
        _ => Language::Unknown,
    }
}

fn discover_plugin_roots(plugins_root: &Path, excludes: &HashSet<String>) -> Result<Vec<PathBuf>> {
    if plugins_root.join("composer.json").is_file() {
        return Ok(vec![plugins_root.to_path_buf()]);
    }

    let mut roots = Vec::new();
    let walker = walk_builder(plugins_root, excludes).build();

    for entry in walker {
        let entry = entry.context("failed while discovering plugin manifests")?;

        if !entry
            .file_type()
            .is_some_and(|file_type| file_type.is_file())
        {
            continue;
        }

        if entry.file_name() == "composer.json"
            && let Some(root) = entry.path().parent()
        {
            roots.push(root.to_path_buf());
        }
    }

    roots.sort();
    roots.dedup();
    roots = without_nested_roots(roots);

    if roots.is_empty() {
        roots.push(plugins_root.to_path_buf());
    }

    Ok(roots)
}

fn without_nested_roots(roots: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut top_level = Vec::new();

    for root in roots {
        if !top_level
            .iter()
            .any(|candidate: &PathBuf| root.starts_with(candidate))
        {
            top_level.push(root);
        }
    }

    top_level
}

fn collect_plugin_candidates(
    plugin: &PluginInfo,
    excludes: &HashSet<String>,
    max_file_size: u64,
    prefilter: &AhoCorasick,
    candidates: &mut Vec<CandidateFile>,
    stats: &mut WalkStats,
) -> Result<()> {
    let walker = walk_builder(&plugin.root, excludes).build();

    for entry in walker {
        let entry =
            entry.with_context(|| format!("failed while walking {}", plugin.root.display()))?;

        if !entry
            .file_type()
            .is_some_and(|file_type| file_type.is_file())
        {
            continue;
        }

        stats.files_seen += 1;

        let language = language_for_path(entry.path());
        if language == Language::Unknown {
            stats.skipped_unsupported += 1;
            continue;
        }

        let metadata = entry
            .metadata()
            .with_context(|| format!("failed to stat {}", entry.path().display()))?;
        let size = metadata.len();

        if size > max_file_size {
            stats.skipped_large += 1;
            continue;
        }

        let relative_path = entry
            .path()
            .strip_prefix(&plugin.root)
            .unwrap_or_else(|_| entry.path())
            .to_path_buf();

        if !is_known_manifest(&relative_path)
            && !matches_prefilter(entry.path(), &relative_path, prefilter)?
        {
            continue;
        }

        candidates.push(CandidateFile {
            plugin: plugin.clone(),
            absolute_path: entry.path().to_path_buf(),
            relative_path,
            language,
            size,
        });
    }

    Ok(())
}

fn walk_builder(root: &Path, excludes: &HashSet<String>) -> WalkBuilder {
    let mut builder = WalkBuilder::new(root);
    let root = root.to_path_buf();
    let excludes = excludes.clone();

    builder
        .hidden(false)
        .parents(true)
        .ignore(true)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(true)
        .filter_entry(move |entry| !is_excluded_entry(entry, &root, &excludes));

    builder
}

fn is_excluded_entry(entry: &DirEntry, root: &Path, excludes: &HashSet<String>) -> bool {
    if entry.path() == root {
        return false;
    }

    entry
        .path()
        .strip_prefix(root)
        .ok()
        .is_some_and(|path| has_excluded_component(path, excludes))
}

fn has_excluded_component(path: &Path, excludes: &HashSet<String>) -> bool {
    path.components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .is_some_and(|component| excludes.contains(component))
    })
}

fn excluded_components(values: &[String]) -> HashSet<String> {
    DEFAULT_EXCLUDES
        .iter()
        .map(|value| (*value).to_owned())
        .chain(
            values
                .iter()
                .map(|value| value.trim().trim_matches(['/', '\\']))
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
        )
        .collect()
}

fn plugin_info_for_root(root: &Path) -> PluginInfo {
    let fallback_name = fallback_plugin_name(root);
    let composer_path = root.join("composer.json");

    let Some(composer) = read_composer_json(&composer_path) else {
        return PluginInfo {
            name: fallback_name,
            version: None,
            root: root.to_path_buf(),
        };
    };

    let name = composer
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or(fallback_name);
    let version = composer
        .get("version")
        .and_then(Value::as_str)
        .filter(|version| !version.trim().is_empty())
        .map(ToOwned::to_owned);

    PluginInfo {
        name,
        version,
        root: root.to_path_buf(),
    }
}

fn read_composer_json(path: &Path) -> Option<Value> {
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn fallback_plugin_name(root: &Path) -> String {
    root.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| root.display().to_string())
}

fn is_known_manifest(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| KNOWN_MANIFESTS.contains(&name))
}

fn matches_prefilter(path: &Path, relative_path: &Path, prefilter: &AhoCorasick) -> Result<bool> {
    if prefilter.is_match(relative_path.to_string_lossy().as_bytes()) {
        return Ok(true);
    }

    let content = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    Ok(prefilter.is_match(&content))
}

fn parse_max_file_size(value: &str) -> Result<u64> {
    let value = value.trim();
    if value.is_empty() {
        return Err(anyhow!("max file size must not be empty"));
    }

    let split_at = value
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(value.len());

    if split_at == 0 {
        return Err(anyhow!("invalid max file size: {value}"));
    }

    let amount = value[..split_at]
        .parse::<u64>()
        .with_context(|| format!("invalid max file size amount: {value}"))?;
    let suffix = value[split_at..].trim().to_ascii_lowercase();
    let multiplier = match suffix.as_str() {
        "" | "b" => 1,
        "kb" => 1_000,
        "mb" => 1_000_000,
        "gb" => 1_000_000_000,
        "kib" => 1_024,
        "mib" => 1_024 * 1_024,
        "gib" => 1_024 * 1_024 * 1_024,
        _ => return Err(anyhow!("unsupported max file size unit: {suffix}")),
    };

    amount
        .checked_mul(multiplier)
        .ok_or_else(|| anyhow!("max file size is too large: {value}"))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    fn index_args(plugins: PathBuf) -> IndexArgs {
        IndexArgs {
            plugins,
            out: PathBuf::from("unused.sqlite"),
            threads: "auto".to_owned(),
            force: false,
            no_snippets: false,
            include_low_confidence: false,
            plugin_manifest: "**/composer.json".to_owned(),
            exclude: vec![
                "vendor".to_owned(),
                "node_modules".to_owned(),
                "dist".to_owned(),
                "build".to_owned(),
                "var".to_owned(),
                "cache".to_owned(),
            ],
            max_file_size: "2MiB".to_owned(),
        }
    }

    #[test]
    fn discovers_plugin_roots_and_parses_composer_metadata() -> Result<()> {
        let temp = tempdir()?;
        let plugins = temp.path().join("plugins");
        let plugin = plugins.join("AcmePlugin");
        fs::create_dir_all(plugin.join("src"))?;
        fs::write(
            plugin.join("composer.json"),
            r#"{"name":"acme/plugin","version":"1.2.3"}"#,
        )?;
        fs::write(plugin.join("manifest.xml"), "<manifest />")?;
        fs::write(
            plugin.join("src/Foo.php"),
            "<?php\nShopware\\Core\\Kernel::class;\n",
        )?;
        fs::write(
            plugin.join("src/NoAnchor.php"),
            "<?php\nclass NoAnchor {}\n",
        )?;

        let args = index_args(plugins);
        let (candidates, stats) = discover_candidates(&args)?;
        let paths = candidate_paths(&candidates);

        assert_eq!(stats.plugins, 1);
        assert_eq!(stats.candidates, 3);
        assert!(paths.contains(&PathBuf::from("composer.json")));
        assert!(paths.contains(&PathBuf::from("manifest.xml")));
        assert!(paths.contains(&PathBuf::from("src/Foo.php")));
        assert!(!paths.contains(&PathBuf::from("src/NoAnchor.php")));
        assert_eq!(candidates[0].plugin.name, "acme/plugin");
        assert_eq!(candidates[0].plugin.version.as_deref(), Some("1.2.3"));

        Ok(())
    }

    #[test]
    fn root_composer_takes_precedence_over_nested_composers() -> Result<()> {
        let temp = tempdir()?;
        let plugins = temp.path().join("plugins");
        fs::create_dir_all(plugins.join("Nested"))?;
        fs::write(plugins.join("composer.json"), r#"{"name":"root/plugin"}"#)?;
        fs::write(
            plugins.join("Nested/composer.json"),
            r#"{"name":"nested/plugin"}"#,
        )?;

        let args = index_args(plugins);
        let (candidates, stats) = discover_candidates(&args)?;

        assert_eq!(stats.plugins, 1);
        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.plugin.name == "root/plugin")
        );

        Ok(())
    }

    #[test]
    fn falls_back_to_plugins_path_when_no_composer_exists() -> Result<()> {
        let temp = tempdir()?;
        let plugins = temp.path().join("plugins");
        fs::create_dir_all(&plugins)?;
        fs::write(plugins.join("theme.json"), r#"{"name":"fallback theme"}"#)?;

        let args = index_args(plugins);
        let (candidates, stats) = discover_candidates(&args)?;

        assert_eq!(stats.plugins, 1);
        assert_eq!(stats.candidates, 1);
        assert_eq!(candidates[0].plugin.name, "plugins");
        assert_eq!(candidates[0].relative_path, PathBuf::from("theme.json"));

        Ok(())
    }

    #[test]
    fn applies_excludes_size_limits_and_language_filtering() -> Result<()> {
        let temp = tempdir()?;
        let plugins = temp.path().join("plugins");
        let plugin = plugins.join("AcmePlugin");
        fs::create_dir_all(plugin.join("src"))?;
        fs::create_dir_all(plugin.join("vendor"))?;
        fs::write(plugin.join("composer.json"), r#"{"name":"acme/plugin"}"#)?;
        fs::write(plugin.join("src/Keep.php"), "<?php Shopware")?;
        fs::write(
            plugin.join("src/Large.php"),
            "<?php Shopware\\Core\\Kernel::class;",
        )?;
        fs::write(plugin.join("src/Unsupported.txt"), "Shopware")?;
        fs::write(plugin.join("vendor/Ignored.php"), "<?php Shopware")?;

        let mut args = index_args(plugins);
        args.max_file_size = "24B".to_owned();
        let (candidates, stats) = discover_candidates(&args)?;
        let paths = candidate_paths(&candidates);

        assert_eq!(stats.files_seen, 4);
        assert_eq!(stats.candidates, 2);
        assert_eq!(stats.skipped_large, 1);
        assert_eq!(stats.skipped_unsupported, 1);
        assert!(paths.contains(&PathBuf::from("composer.json")));
        assert!(paths.contains(&PathBuf::from("src/Keep.php")));
        assert!(!paths.contains(&PathBuf::from("vendor/Ignored.php")));

        Ok(())
    }

    #[test]
    fn parses_file_size_units() -> Result<()> {
        assert_eq!(parse_max_file_size("42")?, 42);
        assert_eq!(parse_max_file_size("10MB")?, 10_000_000);
        assert_eq!(parse_max_file_size("2MiB")?, 2 * 1_024 * 1_024);

        Ok(())
    }

    fn candidate_paths(candidates: &[CandidateFile]) -> Vec<PathBuf> {
        candidates
            .iter()
            .map(|candidate| candidate.relative_path.clone())
            .collect()
    }
}
