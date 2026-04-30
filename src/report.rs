use std::collections::BTreeSet;
use std::fmt::Write;
use std::path::PathBuf;

use crate::model::{ChangedSurface, Confidence};
use crate::source_link::store_plugin_mirror_url;

#[derive(Debug, Clone)]
pub struct ImpactReport {
    pub base: String,
    pub compared: String,
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

pub fn format_human_report(report: &ImpactReport) -> String {
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
    writeln!(output, "Base: {}", report.base).unwrap();
    writeln!(output, "Compared: {}", report.compared).unwrap();
    writeln!(output).unwrap();
    writeln!(output, "Changed surfaces: {}", report.changed_surfaces).unwrap();
    writeln!(output, "Impacted surfaces: {impacted_surfaces}").unwrap();
    writeln!(output, "Affected plugins: {affected_plugins}").unwrap();
    writeln!(output, "Evidence rows shown: {evidence_rows_shown}").unwrap();

    if report.changed_surfaces == 0 {
        writeln!(output).unwrap();
        writeln!(output, "No changed Shopware definition surfaces found.").unwrap();
        return output;
    }

    if report.impacts.is_empty() {
        writeln!(output).unwrap();
        writeln!(output, "{}", no_impact_message()).unwrap();
        return output;
    }

    writeln!(output).unwrap();
    writeln!(
        output,
        "Known extensions touch these changed Shopware surfaces. This is touch evidence, not proof of breakage."
    )
    .unwrap();

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
        writeln!(output, "{}", impact.changed.surface).unwrap();
        writeln!(output, "surface kind: {}", impact.changed.kind.as_str()).unwrap();
        writeln!(output, "change: {}", impact.changed.change.as_str()).unwrap();
        writeln!(output, "confidence: {}", impact.confidence.as_str()).unwrap();
        writeln!(output, "affected plugins: {}", impact.affected_plugins).unwrap();
        writeln!(output, "usages: {}", impact.usage_count).unwrap();

        for evidence in &impact.evidence {
            writeln!(
                output,
                "  {:<24} {:<56} {:<18} {}",
                plugin_label(evidence),
                evidence_location(evidence),
                evidence.usage_kind,
                evidence.confidence.as_str()
            )
            .unwrap();

            if let Some(snippet) = evidence
                .snippet
                .as_deref()
                .filter(|snippet| !snippet.is_empty())
            {
                writeln!(output, "    {}", snippet).unwrap();
            }

            writeln!(output, "    GitHub: {}", evidence.github_url()).unwrap();
        }

        let hidden = impact.usage_count.saturating_sub(impact.evidence.len());
        if hidden > 0 {
            writeln!(output, "  ... {hidden} more evidence row(s) omitted").unwrap();
        }
    }

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
    let mut location = format!("{}:{}", evidence.file_path.display(), evidence.line);

    if let Some(column) = evidence.column {
        write!(location, ":{column}").unwrap();
    }

    location
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
            base: "upstream/trunk".to_owned(),
            compared: "working tree".to_owned(),
            changed_surfaces: 1,
            impacts: Vec::new(),
        };

        let formatted = format_human_report(&report);

        assert!(formatted.contains("Shopware Impact Report"));
        assert!(formatted.contains("Changed surfaces: 1"));
        assert!(formatted.contains(no_impact_message()));
    }

    #[test]
    fn formats_impact_with_truncated_evidence() {
        let report = ImpactReport {
            base: "upstream/trunk".to_owned(),
            compared: "working tree".to_owned(),
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
        assert!(formatted.contains("change: removed"));
        assert!(formatted.contains("affected plugins: 2"));
        assert!(formatted.contains("PluginA@1.2.3"));
        assert!(formatted.contains("... 2 more evidence row(s) omitted"));
    }
}
