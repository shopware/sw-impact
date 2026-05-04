use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{self, IsTerminal};
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use crate::cli::CheckArgs;
use crate::extract;
use crate::git;
use crate::model::{ChangeKind, ChangedSurface, Confidence, Fact, FactRole, Parameter, Signature};
use crate::report::{
    ImpactEvidence, ImpactReport, ReportOptions, SurfaceImpact, format_human_report_with_options,
    sort_impacts,
};
use crate::walker::{Language, language_for_path};

const DEFAULT_BASE: &str = "origin/trunk";

pub fn run_check(args: CheckArgs, verbose: bool) -> Result<()> {
    let started_at = Instant::now();
    let worktree = git::worktree_root(&args.shopware)?;
    let base = resolve_check_base(&worktree, &args.base)?;
    let merge_base = base.revision.as_str();
    let changed_files = git::changed_files(&worktree, merge_base)?;

    if verbose {
        tracing::info!(
            worktree = %worktree.display(),
            changed_files = changed_files.len(),
            "collected changed Shopware files"
        );
    }

    let changed_surfaces = collect_changed_surfaces(&worktree, merge_base, &changed_files)?;
    let mut impacts = lookup_impacts(
        &args.index,
        &changed_surfaces,
        args.include_low_confidence,
        args.only_high_confidence,
        args.max_evidence_per_surface,
    )?;
    sort_impacts(&mut impacts);
    let indexed_plugins = indexed_plugin_count(&args.index)?;

    if verbose {
        tracing::info!(
            changed_surfaces = changed_surfaces.len(),
            impacted_surfaces = impacts.len(),
            "finished impact lookup"
        );
    }

    let report = ImpactReport {
        base: base.label,
        compared: "working tree".to_owned(),
        elapsed: Some(started_at.elapsed()),
        indexed_plugins: Some(indexed_plugins),
        changed_surfaces: changed_surfaces.len(),
        impacts,
    };

    print!(
        "{}",
        format_human_report_with_options(
            &report,
            ReportOptions {
                terminal_links: stdout_supports_links(),
                max_surfaces: Some(args.max_surfaces),
            },
        )
    );
    Ok(())
}

fn stdout_supports_links() -> bool {
    if !io::stdout().is_terminal() {
        return false;
    }

    if env::var_os("SW_IMPACT_NO_LINKS").is_some() {
        return false;
    }

    env::var("TERM").map_or(true, |term| term != "dumb")
}

struct CheckBase {
    revision: String,
    label: String,
}

fn resolve_check_base(worktree: &Path, base: &str) -> Result<CheckBase> {
    match git::resolve_merge_base(worktree, base) {
        Ok(merge_base) => Ok(CheckBase {
            revision: merge_base,
            label: base.to_owned(),
        }),
        Err(error) if base == DEFAULT_BASE => {
            tracing::debug!(
                base,
                error = %error,
                "default check base was unavailable; falling back to HEAD"
            );
            Ok(CheckBase {
                revision: "HEAD".to_owned(),
                label: "HEAD".to_owned(),
            })
        }
        Err(error) => Err(error),
    }
}

fn collect_changed_surfaces(
    worktree: &Path,
    merge_base: &str,
    changed_files: &[std::path::PathBuf],
) -> Result<Vec<ChangedSurface>> {
    let mut base_definitions = Vec::new();
    let mut current_definitions = Vec::new();

    for relative_path in changed_files {
        let language = language_for_path(relative_path);
        if !is_check_definition_language(language) {
            continue;
        }

        if let Some(content) = git::base_file_content(worktree, merge_base, relative_path)? {
            base_definitions.extend(extract_definitions(language, &content, relative_path)?);
        }

        if let Some(content) = current_file_content(worktree, relative_path)? {
            current_definitions.extend(extract_definitions(language, &content, relative_path)?);
        }
    }

    Ok(diff_definitions(&base_definitions, &current_definitions))
}

fn is_check_definition_language(language: Language) -> bool {
    matches!(
        language,
        Language::Php
            | Language::Twig
            | Language::Json
            | Language::Yaml
            | Language::Toml
            | Language::JavaScript
            | Language::TypeScript
            | Language::Vue
    )
}

fn extract_definitions(
    language: Language,
    content: &str,
    relative_path: &Path,
) -> Result<Vec<Fact>> {
    let facts =
        extract::extract_facts(language, content, relative_path, FactRole::Definition, true)
            .with_context(|| format!("failed to extract facts from {}", relative_path.display()))?;

    Ok(facts
        .into_iter()
        .filter(|fact| fact.role == FactRole::Definition)
        .collect())
}

fn current_file_content(worktree: &Path, relative_path: &Path) -> Result<Option<String>> {
    let path = worktree.join(relative_path);

    if !path.is_file() {
        return Ok(None);
    }

    fs::read_to_string(&path)
        .map(Some)
        .with_context(|| format!("failed to read current content for {}", path.display()))
}

fn diff_definitions(base: &[Fact], current: &[Fact]) -> Vec<ChangedSurface> {
    let base = definition_map(base);
    let current = definition_map(current);
    let mut changes = Vec::new();

    for (surface, base_fact) in &base {
        match current.get(surface) {
            Some(current_fact) => {
                if let Some(change) = signature_change(
                    base_fact.signature.as_ref(),
                    current_fact.signature.as_ref(),
                ) {
                    changes.push(changed_surface(
                        current_fact,
                        change,
                        std::cmp::min(base_fact.confidence, current_fact.confidence),
                    ));
                }
            }
            None => changes.push(changed_surface(
                base_fact,
                ChangeKind::Removed,
                base_fact.confidence,
            )),
        }
    }

    for (surface, current_fact) in &current {
        if !base.contains_key(surface) {
            changes.push(changed_surface(
                current_fact,
                ChangeKind::DefinitionAdded,
                current_fact.confidence,
            ));
        }
    }

    changes.sort_by(|left, right| {
        left.surface
            .as_str()
            .cmp(right.surface.as_str())
            .then_with(|| left.change.as_str().cmp(right.change.as_str()))
    });
    changes
}

fn definition_map(facts: &[Fact]) -> BTreeMap<String, Fact> {
    let mut definitions = BTreeMap::new();

    for fact in facts {
        definitions
            .entry(fact.surface.as_str().to_owned())
            .and_modify(|existing| {
                if better_definition(fact, existing) {
                    *existing = fact.clone();
                }
            })
            .or_insert_with(|| fact.clone());
    }

    definitions
}

fn better_definition(candidate: &Fact, existing: &Fact) -> bool {
    candidate.confidence > existing.confidence
        || (candidate.confidence == existing.confidence
            && candidate.signature.is_some()
            && existing.signature.is_none())
}

fn changed_surface(fact: &Fact, change: ChangeKind, confidence: Confidence) -> ChangedSurface {
    ChangedSurface {
        surface: fact.surface.clone(),
        kind: fact.kind,
        change,
        confidence,
        evidence: fact.evidence.clone(),
    }
}

fn signature_change(base: Option<&Signature>, current: Option<&Signature>) -> Option<ChangeKind> {
    let (Some(base), Some(current)) = (base, current) else {
        return None;
    };

    if base == current {
        return None;
    }

    if base.visibility != current.visibility {
        return Some(ChangeKind::VisibilityChanged);
    }

    if current.parameters.len() < base.parameters.len() {
        return Some(ChangeKind::ParameterRemoved);
    }

    if required_parameter_added(&base.parameters, &current.parameters) {
        return Some(ChangeKind::RequiredParameterAdded);
    }

    if base.return_type != current.return_type {
        return Some(ChangeKind::ReturnTypeChanged);
    }

    Some(ChangeKind::SignatureChanged)
}

fn required_parameter_added(base: &[Parameter], current: &[Parameter]) -> bool {
    current.iter().enumerate().any(|(index, parameter)| {
        parameter.required
            && base
                .get(index)
                .map(|base_parameter| !base_parameter.required)
                .unwrap_or(true)
    })
}

fn lookup_impacts(
    index_path: &Path,
    changed_surfaces: &[ChangedSurface],
    include_low_confidence: bool,
    only_high_confidence: bool,
    max_evidence_per_surface: usize,
) -> Result<Vec<SurfaceImpact>> {
    let connection = Connection::open(index_path)
        .with_context(|| format!("failed to open index {}", index_path.display()))?;
    let min_confidence = min_confidence(include_low_confidence, only_high_confidence);
    let mut impacts = Vec::new();

    for changed in changed_surfaces {
        if let Some(impact) = lookup_surface_impact(
            &connection,
            changed,
            min_confidence,
            max_evidence_per_surface,
        )? {
            impacts.push(impact);
        }
    }

    Ok(impacts)
}

fn indexed_plugin_count(index_path: &Path) -> Result<usize> {
    let connection = Connection::open(index_path)
        .with_context(|| format!("failed to open index {}", index_path.display()))?;
    let count: i64 = connection
        .query_row("select count(*) from plugin", [], |row| row.get(0))
        .context("failed to query indexed plugin count")?;

    non_negative_usize(count, "plugin count")
}

fn lookup_surface_impact(
    connection: &Connection,
    changed: &ChangedSurface,
    min_confidence: i64,
    max_evidence_per_surface: usize,
) -> Result<Option<SurfaceImpact>> {
    let key = changed.surface.as_str();
    let (usage_count, affected_plugins, confidence) =
        impact_totals(connection, key, min_confidence)?;

    if usage_count == 0 {
        return Ok(None);
    }

    let affected_plugin_ids = affected_plugin_ids(connection, key, min_confidence)?;
    let evidence = impact_evidence(connection, key, min_confidence, max_evidence_per_surface)?;

    Ok(Some(SurfaceImpact {
        changed: changed.clone(),
        confidence,
        affected_plugins,
        affected_plugin_ids,
        usage_count,
        evidence,
    }))
}

fn impact_totals(
    connection: &Connection,
    surface: &str,
    min_confidence: i64,
) -> Result<(usize, usize, Confidence)> {
    let (usage_count, affected_plugins, confidence): (i64, i64, Option<i64>) = connection
        .query_row(
            r#"
            select count(*) as usage_count,
                   count(distinct e.plugin_id) as affected_plugins,
                   max(e.confidence) as confidence
            from surface s
            join evidence e on e.surface_id = s.id
            where s.key = ?1 and e.confidence >= ?2
            "#,
            params![surface, min_confidence],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .with_context(|| format!("failed to query impact totals for {surface}"))?;

    Ok((
        non_negative_usize(usage_count, "usage_count")?,
        non_negative_usize(affected_plugins, "affected_plugins")?,
        Confidence::from_i64(confidence.unwrap_or(1)),
    ))
}

fn affected_plugin_ids(
    connection: &Connection,
    surface: &str,
    min_confidence: i64,
) -> Result<Vec<i64>> {
    let mut statement = connection
        .prepare(
            r#"
            select distinct e.plugin_id
            from surface s
            join evidence e on e.surface_id = s.id
            where s.key = ?1 and e.confidence >= ?2
            order by e.plugin_id
            "#,
        )
        .with_context(|| format!("failed to prepare affected plugin query for {surface}"))?;
    let rows = statement.query_map(params![surface, min_confidence], |row| row.get(0))?;
    let mut ids = Vec::new();

    for row in rows {
        ids.push(row?);
    }

    Ok(ids)
}

fn impact_evidence(
    connection: &Connection,
    surface: &str,
    min_confidence: i64,
    max_evidence_per_surface: usize,
) -> Result<Vec<ImpactEvidence>> {
    let limit = i64::try_from(max_evidence_per_surface).unwrap_or(i64::MAX);
    let mut statement = connection
        .prepare(
            r#"
            select e.plugin_id,
                   p.name,
                   p.version,
                   p.path,
                   indexed_file.path,
                   e.line,
                   e.column,
                   e.usage_kind,
                   e.snippet,
                   e.confidence
            from surface s
            join evidence e on e.surface_id = s.id
            join plugin p on p.id = e.plugin_id
            join file indexed_file on indexed_file.id = e.file_id
            where s.key = ?1 and e.confidence >= ?2
            order by e.confidence desc,
                     p.name asc,
                     indexed_file.path asc,
                     e.line asc,
                     coalesce(e.column, 0) asc,
                     e.usage_kind asc
            limit ?3
            "#,
        )
        .with_context(|| format!("failed to prepare evidence query for {surface}"))?;

    let rows = statement.query_map(params![surface, min_confidence, limit], |row| {
        Ok(RawEvidence {
            plugin_id: row.get(0)?,
            plugin: row.get(1)?,
            plugin_version: row.get(2)?,
            plugin_path: row.get(3)?,
            file_path: row.get(4)?,
            line: row.get(5)?,
            column: row.get(6)?,
            usage_kind: row.get(7)?,
            snippet: row.get(8)?,
            confidence: row.get(9)?,
        })
    })?;
    let mut evidence = Vec::new();

    for row in rows {
        let raw = row?;
        evidence.push(raw.try_into()?);
    }

    Ok(evidence)
}

fn min_confidence(include_low_confidence: bool, only_high_confidence: bool) -> i64 {
    if only_high_confidence {
        Confidence::High as i64
    } else if include_low_confidence {
        Confidence::Low as i64
    } else {
        Confidence::Medium as i64
    }
}

fn non_negative_usize(value: i64, field: &str) -> Result<usize> {
    usize::try_from(value).with_context(|| format!("{field} is negative or too large: {value}"))
}

struct RawEvidence {
    plugin_id: i64,
    plugin: String,
    plugin_version: Option<String>,
    plugin_path: String,
    file_path: String,
    line: i64,
    column: Option<i64>,
    usage_kind: String,
    snippet: Option<String>,
    confidence: i64,
}

impl TryFrom<RawEvidence> for ImpactEvidence {
    type Error = anyhow::Error;

    fn try_from(raw: RawEvidence) -> Result<Self> {
        Ok(Self {
            plugin_id: raw.plugin_id,
            plugin: raw.plugin,
            plugin_version: raw.plugin_version,
            plugin_path: raw.plugin_path.into(),
            file_path: raw.file_path.into(),
            line: non_negative_usize(raw.line, "line")?,
            column: raw
                .column
                .map(|column| non_negative_usize(column, "column"))
                .transpose()?,
            usage_kind: raw.usage_kind,
            snippet: raw.snippet,
            confidence: Confidence::from_i64(raw.confidence),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::config::extract_json;
    use crate::model::{Evidence, Parameter};

    fn evidence(path: &str, line: usize) -> Evidence {
        Evidence {
            path: path.into(),
            line,
            column: None,
            snippet: None,
        }
    }

    fn signature(parameters: Vec<Parameter>) -> Signature {
        Signature {
            visibility: Some("public".to_owned()),
            parameters,
            return_type: Some("void".to_owned()),
        }
    }

    fn parameter(name: &str, required: bool) -> Parameter {
        Parameter {
            name: name.to_owned(),
            type_name: Some("string".to_owned()),
            required,
        }
    }

    fn definition(surface: &str, signature: Option<Signature>) -> Fact {
        Fact::definition(
            surface,
            evidence("src/Core/Foo.php", 10),
            Confidence::High,
            "definition",
            signature,
        )
    }

    #[test]
    fn diff_detects_removed_definition() {
        let base = vec![definition("php:method:Shopware\\Core\\Foo::removed", None)];
        let current = Vec::new();

        let changes = diff_definitions(&base, &current);

        assert_eq!(changes.len(), 1);
        assert_eq!(
            changes[0].surface.as_str(),
            "php:method:Shopware\\Core\\Foo::removed"
        );
        assert_eq!(changes[0].change, ChangeKind::Removed);
    }

    #[test]
    fn diff_detects_required_parameter_added() {
        let base = vec![definition(
            "php:method:Shopware\\Core\\Foo::bar",
            Some(signature(vec![parameter("id", true)])),
        )];
        let current = vec![definition(
            "php:method:Shopware\\Core\\Foo::bar",
            Some(signature(vec![
                parameter("id", true),
                parameter("context", true),
            ])),
        )];

        let changes = diff_definitions(&base, &current);

        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].change, ChangeKind::RequiredParameterAdded);
    }

    #[test]
    fn confidence_filter_defaults_to_medium() {
        assert_eq!(min_confidence(false, false), Confidence::Medium as i64);
        assert_eq!(min_confidence(true, false), Confidence::Low as i64);
        assert_eq!(min_confidence(true, true), Confidence::High as i64);
    }

    #[test]
    fn snippet_value_changes_are_not_changed_surfaces_but_removed_keys_are() {
        let base = snippet_definitions(
            r#"{
  "sw-order": {
    "general": {
      "title": "Orders",
      "subtitle": "Overview"
    }
  }
}"#,
        );
        let changed_value = snippet_definitions(
            r#"{
  "sw-order": {
    "general": {
      "title": "Updated orders",
      "subtitle": "Overview"
    }
  }
}"#,
        );
        let removed_key = snippet_definitions(
            r#"{
  "sw-order": {
    "general": {
      "title": "Updated orders"
    }
  }
}"#,
        );

        assert!(diff_definitions(&base, &changed_value).is_empty());

        let changes = diff_definitions(&changed_value, &removed_key);
        assert_eq!(changes.len(), 1);
        assert_eq!(
            changes[0].surface.as_str(),
            "snippet:key:sw-order.general.subtitle"
        );
        assert_eq!(changes[0].change, ChangeKind::Removed);
    }

    fn snippet_definitions(content: &str) -> Vec<Fact> {
        extract_json(
            content,
            Path::new("Resources/snippet/en-GB/storefront.en-GB.json"),
            FactRole::Definition,
            false,
        )
        .expect("snippet json extracts")
    }
}
