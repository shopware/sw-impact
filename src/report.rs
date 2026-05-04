use std::collections::BTreeSet;
use std::fmt::Write;
use std::path::PathBuf;
use std::time::Duration;

use crate::model::{ChangeKind, ChangedSurface, Confidence};
use crate::source_link::store_plugin_mirror_url;

#[derive(Debug, Clone)]
pub struct ImpactReport {
    pub base: String,
    pub compared: String,
    pub elapsed: Option<Duration>,
    pub indexed_plugins: Option<usize>,
    pub changed_surfaces: usize,
    pub impacts: Vec<SurfaceImpact>,
}

#[derive(Debug, Clone)]
pub struct SurfaceImpact {
    pub changed: ChangedSurface,
    pub confidence: Confidence,
    pub affected_plugins: usize,
    pub affected_plugin_ids: Vec<i64>,
    pub usage_count: usize,
    pub evidence: Vec<ImpactEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImpactEvidence {
    pub plugin_id: i64,
    pub plugin: String,
    pub plugin_version: Option<String>,
    pub plugin_path: PathBuf,
    pub file_path: PathBuf,
    pub line: usize,
    pub column: Option<usize>,
    pub usage_kind: String,
    pub snippet: Option<String>,
    pub confidence: Confidence,
}

pub fn no_impact_message() -> &'static str {
    "No indexed plugin impact found for changed surfaces."
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ReportOptions {
    pub terminal_links: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct EvidenceDisplay {
    pub plugin: String,
    pub file: String,
    pub usage: String,
    pub confidence: Confidence,
    pub source_url: String,
    pub snippet: Option<String>,
}

pub(crate) const SEPARATOR: &str =
    "--------------------------------------------------------------------------------";

pub fn format_human_report(report: &ImpactReport) -> String {
    format_human_report_with_options(report, ReportOptions::default())
}

pub fn format_human_report_with_options(report: &ImpactReport, options: ReportOptions) -> String {
    let mut output = String::new();
    let impacted_surfaces = report.impacts.len();
    let affected_plugins = report
        .impacts
        .iter()
        .flat_map(|impact| impact.affected_plugin_ids.iter().copied())
        .collect::<BTreeSet<_>>()
        .len();
    let evidence_rows_shown: usize = report
        .impacts
        .iter()
        .map(|impact| impact.evidence.len())
        .sum();

    writeln!(output, "Shopware Impact Report").unwrap();

    if report.changed_surfaces == 0 {
        writeln!(output).unwrap();
        writeln!(output, "No changed Shopware definition surfaces found.").unwrap();
        write_summary(
            &mut output,
            report,
            impacted_surfaces,
            affected_plugins,
            evidence_rows_shown,
        );
        return output;
    }

    if report.impacts.is_empty() {
        writeln!(output).unwrap();
        writeln!(output, "{}", no_impact_message()).unwrap();
        write_summary(
            &mut output,
            report,
            impacted_surfaces,
            affected_plugins,
            evidence_rows_shown,
        );
        return output;
    }

    let mut last_confidence = None;
    for impact in &report.impacts {
        if last_confidence != Some(impact.confidence) {
            writeln!(output).unwrap();
            writeln!(
                output,
                "{} confidence impact",
                title_case(impact.confidence.as_str())
            )
            .unwrap();
            last_confidence = Some(impact.confidence);
        }

        writeln!(output).unwrap();
        write_surface_header(&mut output, impact);
        write_impact_evidence_entries(&mut output, impact, options);
        let hidden = impact.usage_count.saturating_sub(impact.evidence.len());
        if hidden > 0 {
            writeln!(output, "  ... {hidden} more evidence row(s) omitted").unwrap();
        }
    }

    writeln!(output).unwrap();
    writeln!(
        output,
        "Known extensions touch these changed Shopware surfaces. This is touch evidence, not proof of breakage."
    )
    .unwrap();
    write_summary(
        &mut output,
        report,
        impacted_surfaces,
        affected_plugins,
        evidence_rows_shown,
    );

    output
}

pub fn sort_impacts(impacts: &mut [SurfaceImpact]) {
    impacts.sort_by(|left, right| {
        right
            .confidence
            .cmp(&left.confidence)
            .then_with(|| right.affected_plugins.cmp(&left.affected_plugins))
            .then_with(|| right.usage_count.cmp(&left.usage_count))
            .then_with(|| {
                left.changed
                    .surface
                    .as_str()
                    .cmp(right.changed.surface.as_str())
            })
    });
}

fn plugin_label(evidence: &ImpactEvidence) -> String {
    match evidence.plugin_version.as_deref() {
        Some(version) if !version.is_empty() => format!("{}@{version}", evidence.plugin),
        _ => evidence.plugin.clone(),
    }
}

fn evidence_location(evidence: &ImpactEvidence) -> String {
    format!("{}:{}", evidence.file_path.display(), evidence.line)
}

pub(crate) fn write_numbered_evidence_entries(
    output: &mut String,
    evidence: &[EvidenceDisplay],
    options: ReportOptions,
) {
    if evidence.is_empty() {
        writeln!(output, "  (no evidence rows shown)").unwrap();
        return;
    }

    for (index, evidence) in evidence.iter().enumerate() {
        let source = if options.terminal_links {
            terminal_link("GitHub ↗", &evidence.source_url)
        } else {
            format!("GitHub ↗ {}", evidence.source_url)
        };

        writeln!(output, "  {}. {}", index + 1, evidence.plugin).unwrap();
        let mut rows = vec![
            ("File", evidence.file.clone()),
            ("Usage", evidence.usage.clone()),
            ("Confidence", evidence.confidence.as_str().to_owned()),
            ("Source", source),
        ];
        if let Some(snippet) = evidence
            .snippet
            .as_deref()
            .filter(|snippet| !snippet.is_empty())
        {
            rows.push(("Snippet", snippet.to_owned()));
        }
        write_aligned_key_values(output, "     ", &rows);
    }
}

fn write_impact_evidence_entries(
    output: &mut String,
    impact: &SurfaceImpact,
    options: ReportOptions,
) {
    let evidence = impact
        .evidence
        .iter()
        .map(|evidence| EvidenceDisplay {
            plugin: plugin_label(evidence),
            file: evidence_location(evidence),
            usage: evidence.usage_kind.clone(),
            confidence: evidence.confidence,
            source_url: evidence.github_url(),
            snippet: evidence.snippet.clone(),
        })
        .collect::<Vec<_>>();

    write_numbered_evidence_entries(output, &evidence, options);
}

fn write_surface_header(output: &mut String, impact: &SurfaceImpact) {
    let mut rows = vec![
        ("surface kind", impact.changed.kind.as_str().to_owned()),
        ("change", impact.changed.change.as_str().to_owned()),
        ("shopware source", changed_surface_location(&impact.changed)),
        ("confidence", impact.confidence.as_str().to_owned()),
        ("affected plugins", impact.affected_plugins.to_string()),
        ("usages", impact.usage_count.to_string()),
    ];
    if let Some(snippet) = impact
        .changed
        .evidence
        .snippet
        .as_deref()
        .filter(|snippet| !snippet.is_empty())
    {
        rows.push(("shopware snippet", snippet.to_owned()));
    }
    write_dashed_block(output, impact.changed.surface.as_str(), &rows);
}

fn write_summary(
    output: &mut String,
    report: &ImpactReport,
    impacted_surfaces: usize,
    affected_plugins: usize,
    evidence_rows_shown: usize,
) {
    writeln!(output).unwrap();
    writeln!(output, "{SEPARATOR}").unwrap();
    writeln!(output, "Summary").unwrap();
    let mut rows = vec![
        ("Base", report.base.clone()),
        ("Compared", report.compared.clone()),
    ];
    if let Some(elapsed) = report.elapsed {
        rows.push(("Runtime", format_duration(elapsed)));
    }
    rows.extend([
        ("Changed surfaces", report.changed_surfaces.to_string()),
        ("Impacted surfaces", impacted_surfaces.to_string()),
        ("Evidence rows shown", evidence_rows_shown.to_string()),
        (
            "Affected plugins",
            affected_plugins_summary(affected_plugins, report.indexed_plugins),
        ),
    ]);
    write_aligned_key_values(output, "", &rows);
    writeln!(output, "{SEPARATOR}").unwrap();
}

pub(crate) fn terminal_link(label: &str, url: &str) -> String {
    format!("\x1b]8;;{url}\x1b\\{label}\x1b]8;;\x1b\\")
}

pub(crate) fn write_dashed_block(output: &mut String, title: &str, rows: &[(&str, String)]) {
    writeln!(output, "{SEPARATOR}").unwrap();
    writeln!(output, "{title}").unwrap();
    write_aligned_key_values(output, "  ", rows);
    writeln!(output, "{SEPARATOR}").unwrap();
}

pub(crate) fn affected_plugins_summary(
    affected_plugins: usize,
    indexed_plugins: Option<usize>,
) -> String {
    let Some(indexed_plugins) = indexed_plugins else {
        return affected_plugins.to_string();
    };

    if indexed_plugins == 0 {
        return format!("{affected_plugins} / 0");
    }

    let percentage = affected_plugins as f64 * 100.0 / indexed_plugins as f64;
    format!("{affected_plugins} / {indexed_plugins} ({percentage:.1}%)")
}

pub(crate) fn write_aligned_key_values(output: &mut String, indent: &str, rows: &[(&str, String)]) {
    let key_width = rows
        .iter()
        .map(|(key, _)| key.chars().count())
        .max()
        .unwrap_or(0);

    for (key, value) in rows {
        let padding = key_width.saturating_sub(key.chars().count()) + 1;
        writeln!(output, "{indent}{key}:{}{}", " ".repeat(padding), value).unwrap();
    }
}

fn format_duration(duration: Duration) -> String {
    let total_millis = duration.as_millis();
    let minutes = total_millis / 60_000;
    let seconds = (total_millis % 60_000) / 1_000;
    let millis = total_millis % 1_000;

    if minutes > 0 {
        format!("{minutes}m {seconds}.{millis:03}s")
    } else {
        format!("{seconds}.{millis:03}s")
    }
}

fn changed_surface_location(changed: &ChangedSurface) -> String {
    let mut location = format!(
        "{}:{}",
        changed.evidence.path.display(),
        changed.evidence.line
    );

    if let Some(column) = changed.evidence.column {
        write!(location, ":{column}").unwrap();
    }

    let tree = match &changed.change {
        ChangeKind::Removed => "base",
        _ => "working tree",
    };

    format!("{location} ({tree})")
}

impl ImpactEvidence {
    fn github_url(&self) -> String {
        store_plugin_mirror_url(
            &self.plugin_path.to_string_lossy(),
            &self.file_path,
            self.line,
        )
    }
}

fn title_case(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ChangeKind, Evidence, SurfaceKey};

    fn changed(surface: &str) -> ChangedSurface {
        let surface = SurfaceKey::new(surface);
        ChangedSurface {
            kind: surface.kind(),
            surface,
            change: ChangeKind::Removed,
            confidence: Confidence::High,
            evidence: Evidence {
                path: PathBuf::from("src/Core/Foo.php"),
                line: 10,
                column: Some(5),
                snippet: Some("public function removed(): void".to_owned()),
            },
        }
    }

    #[test]
    fn formats_no_impact_report() {
        let report = ImpactReport {
            base: "origin/trunk".to_owned(),
            compared: "working tree".to_owned(),
            elapsed: Some(Duration::from_millis(123)),
            indexed_plugins: Some(2_000),
            changed_surfaces: 1,
            impacts: Vec::new(),
        };

        let formatted = format_human_report(&report);

        assert!(formatted.contains("Shopware Impact Report"));
        assert!(formatted.contains("Runtime:             0.123s"));
        assert!(formatted.contains("Changed surfaces:    1"));
        assert!(formatted.contains("Affected plugins:    0 / 2000 (0.0%)"));
        assert!(formatted.contains(no_impact_message()));
    }

    #[test]
    fn formats_impact_with_numbered_evidence() {
        let report = ImpactReport {
            base: "origin/trunk".to_owned(),
            compared: "working tree".to_owned(),
            elapsed: Some(Duration::from_millis(123)),
            indexed_plugins: Some(2_000),
            changed_surfaces: 1,
            impacts: vec![SurfaceImpact {
                changed: changed("php:method:Shopware\\Core\\Foo::bar"),
                confidence: Confidence::High,
                affected_plugins: 2,
                affected_plugin_ids: vec![1, 2],
                usage_count: 3,
                evidence: vec![ImpactEvidence {
                    plugin_id: 1,
                    plugin: "PluginA".to_owned(),
                    plugin_version: Some("1.2.3".to_owned()),
                    plugin_path: PathBuf::from("PluginA"),
                    file_path: PathBuf::from("src/Foo.php"),
                    line: 42,
                    column: None,
                    usage_kind: "method-call".to_owned(),
                    snippet: Some("$service->bar();".to_owned()),
                    confidence: Confidence::High,
                }],
            }],
        };

        let formatted = format_human_report(&report);

        assert!(formatted.contains("High confidence impact"));
        assert!(formatted.contains("php:method:Shopware\\Core\\Foo::bar"));
        assert!(formatted.contains(SEPARATOR));
        assert!(formatted.contains("  1. PluginA@1.2.3"));
        assert!(formatted.contains("     File:       src/Foo.php:42"));
        assert!(formatted.contains("  change:           removed"));
        assert!(formatted.contains("  shopware source:  src/Core/Foo.php:10:5 (base)"));
        assert!(formatted.contains("public function removed(): void"));
        assert!(formatted.contains("affected plugins: 2"));
        assert!(formatted.contains("Summary"));
        assert!(formatted.contains("Runtime:             0.123s"));
        assert!(formatted.contains("Affected plugins:    2 / 2000 (0.1%)"));
        assert!(formatted.contains("... 2 more evidence row(s) omitted"));
    }

    #[test]
    fn formats_terminal_links_when_enabled() {
        let evidence = ImpactEvidence {
            plugin_id: 1,
            plugin: "PluginA".to_owned(),
            plugin_version: None,
            plugin_path: PathBuf::from("PluginA"),
            file_path: PathBuf::from("src/Foo.php"),
            line: 42,
            column: None,
            usage_kind: "method-call".to_owned(),
            snippet: None,
            confidence: Confidence::High,
        };
        let report = ImpactReport {
            base: "origin/trunk".to_owned(),
            compared: "working tree".to_owned(),
            elapsed: None,
            indexed_plugins: Some(2_000),
            changed_surfaces: 1,
            impacts: vec![SurfaceImpact {
                changed: changed("php:method:Shopware\\Core\\Foo::bar"),
                confidence: Confidence::High,
                affected_plugins: 1,
                affected_plugin_ids: vec![1],
                usage_count: 1,
                evidence: vec![evidence],
            }],
        };

        let formatted = format_human_report_with_options(
            &report,
            ReportOptions {
                terminal_links: true,
            },
        );

        assert!(formatted.contains("\x1b]8;;https://github.com/"));
        assert!(formatted.contains("\x1b\\GitHub ↗\x1b]8;;\x1b\\"));
    }
}
