use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;

use crate::model::{Confidence, Evidence, Fact, FactRole, line_col_at, line_snippet};

pub fn extract(
    content: &str,
    relative_path: &Path,
    role: FactRole,
    include_snippets: bool,
) -> Vec<Fact> {
    let mut facts = FactCollector::new();

    if role == FactRole::Definition
        && let Some(template) = template_name_from_path(relative_path)
    {
        facts.push(
            role,
            format!("twig:template:{template}"),
            evidence(content, relative_path, 0, include_snippets),
            Confidence::High,
            "twig.template.definition",
        );
    }

    extract_blocks(content, relative_path, role, include_snippets, &mut facts);
    extract_template_references(content, relative_path, role, include_snippets, &mut facts);
    extract_route_references(content, relative_path, role, include_snippets, &mut facts);

    facts.into_vec()
}

fn extract_blocks(
    content: &str,
    relative_path: &Path,
    role: FactRole,
    include_snippets: bool,
    facts: &mut FactCollector,
) {
    for captures in block_re().captures_iter(content) {
        let Some(block_match) = captures.get(1) else {
            continue;
        };

        let usage_kind = if role == FactRole::Definition {
            "twig.block.definition"
        } else {
            "twig.block.usage"
        };

        facts.push(
            role,
            format!("twig:block:{}", block_match.as_str()),
            evidence(
                content,
                relative_path,
                block_match.start(),
                include_snippets,
            ),
            Confidence::High,
            usage_kind,
        );
    }
}

fn extract_template_references(
    content: &str,
    relative_path: &Path,
    role: FactRole,
    include_snippets: bool,
    facts: &mut FactCollector,
) {
    for captures in template_tag_re().captures_iter(content) {
        let Some(tag_match) = captures.get(1) else {
            continue;
        };
        let Some(body_match) = captures.get(2) else {
            continue;
        };

        for string_captures in string_re().captures_iter(body_match.as_str()) {
            let Some(template_match) = string_captures.get(1).or_else(|| string_captures.get(2))
            else {
                continue;
            };
            let template = template_match.as_str();

            if !looks_like_twig_template(template) {
                continue;
            }

            let offset = body_match.start() + template_match.start();
            facts.push(
                role,
                format!("twig:template:{template}"),
                evidence(content, relative_path, offset, include_snippets),
                Confidence::High,
                format!("twig.template.{}", tag_match.as_str()),
            );
        }
    }
}

fn extract_route_references(
    content: &str,
    relative_path: &Path,
    role: FactRole,
    include_snippets: bool,
    facts: &mut FactCollector,
) {
    for captures in route_re().captures_iter(content) {
        let Some(function_match) = captures.get(1) else {
            continue;
        };
        let Some(route_match) = captures.get(2).or_else(|| captures.get(3)) else {
            continue;
        };

        if !looks_like_route_name(route_match.as_str()) {
            continue;
        }

        facts.push(
            role,
            format!("route:name:{}", route_match.as_str()),
            evidence(
                content,
                relative_path,
                route_match.start(),
                include_snippets,
            ),
            Confidence::High,
            format!("twig.route.{}", function_match.as_str()),
        );
    }
}

fn block_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\{%-?\s*block\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap())
}

fn template_tag_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?s)\{%-?\s*(sw_extends|extends|sw_include|include)\b(.*?)%}").unwrap()
    })
}

fn string_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"'([^']+)'|"([^"]+)""#).unwrap())
}

fn route_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"\b(path|seoUrl)\s*\(\s*(?:'([^']+)'|"([^"]+)")"#).unwrap())
}

fn template_name_from_path(relative_path: &Path) -> Option<String> {
    let normalized = relative_path.to_string_lossy().replace('\\', "/");

    for (marker, namespace) in [
        ("/Storefront/Resources/views/", "@Storefront/"),
        ("/Administration/Resources/views/", "@Administration/"),
        ("Storefront/Resources/views/", "@Storefront/"),
        ("Administration/Resources/views/", "@Administration/"),
    ] {
        if let Some(index) = normalized.find(marker) {
            return Some(format!(
                "{namespace}{}",
                &normalized[index + marker.len()..]
            ));
        }
    }

    if normalized.ends_with(".twig") {
        Some(normalized)
    } else {
        None
    }
}

fn looks_like_twig_template(value: &str) -> bool {
    let trimmed = value.trim();
    (trimmed.starts_with('@') || trimmed.ends_with(".twig") || trimmed.contains(".html.twig"))
        && !trimmed.contains("{{")
        && !trimmed.contains("{%")
}

fn looks_like_route_name(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed.contains('.')
        && trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}

fn evidence(
    content: &str,
    relative_path: &Path,
    byte_offset: usize,
    include_snippets: bool,
) -> Evidence {
    let (line, column) = line_col_at(content, byte_offset.min(content.len()));

    Evidence {
        path: relative_path.to_path_buf(),
        line,
        column: Some(column),
        snippet: include_snippets
            .then(|| line_snippet(content, line))
            .flatten(),
    }
}

struct FactCollector {
    facts: Vec<Fact>,
}

impl FactCollector {
    fn new() -> Self {
        Self { facts: Vec::new() }
    }

    fn push(
        &mut self,
        role: FactRole,
        surface: String,
        evidence: Evidence,
        confidence: Confidence,
        usage_kind: impl Into<String>,
    ) {
        let usage_kind = usage_kind.into();
        let duplicate = self.facts.iter().any(|fact| {
            fact.surface.as_str() == surface
                && fact.role == role
                && fact.usage_kind == usage_kind
                && fact.evidence.line == evidence.line
                && fact.evidence.column == evidence.column
        });

        if duplicate {
            return;
        }

        let fact = match role {
            FactRole::Usage => Fact::usage(surface, evidence, confidence, usage_kind),
            FactRole::Definition => {
                Fact::definition(surface, evidence, confidence, usage_kind, None)
            }
        };
        self.facts.push(fact);
    }

    fn into_vec(self) -> Vec<Fact> {
        self.facts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{FactRole, SurfaceKind};

    fn surfaces(facts: &[Fact]) -> Vec<String> {
        facts
            .iter()
            .map(|fact| fact.surface.as_str().to_string())
            .collect()
    }

    #[test]
    fn extracts_twig_usages() {
        let content = r#"
{% sw_extends '@Storefront/storefront/base.html.twig' %}
{% block storefront_page_product_detail_buy %}
    {% sw_include '@Storefront/storefront/component/buy-widget.html.twig' %}
    <a href="{{ path('frontend.detail.page') }}"></a>
    <a href="{{ seoUrl('frontend.navigation.page') }}"></a>
{% endblock %}
"#;

        let facts = extract(
            content,
            Path::new("src/Resources/views/storefront/page/detail.html.twig"),
            FactRole::Usage,
            true,
        );
        let surfaces = surfaces(&facts);

        assert!(
            surfaces.contains(&"twig:template:@Storefront/storefront/base.html.twig".to_string())
        );
        assert!(surfaces.contains(
            &"twig:template:@Storefront/storefront/component/buy-widget.html.twig".to_string()
        ));
        assert!(surfaces.contains(&"twig:block:storefront_page_product_detail_buy".to_string()));
        assert!(surfaces.contains(&"route:name:frontend.detail.page".to_string()));
        assert!(surfaces.contains(&"route:name:frontend.navigation.page".to_string()));
        assert!(facts.iter().all(|fact| fact.role == FactRole::Usage));
        assert!(facts.iter().any(|fact| fact.evidence.snippet.is_some()));
    }

    #[test]
    fn extracts_twig_definitions_for_template_and_blocks() {
        let content = "{% block base_header %}{% endblock %}";
        let facts = extract(
            content,
            Path::new("src/Storefront/Resources/views/storefront/base.html.twig"),
            FactRole::Definition,
            false,
        );
        let surfaces = surfaces(&facts);

        assert!(
            surfaces.contains(&"twig:template:@Storefront/storefront/base.html.twig".to_string())
        );
        assert!(surfaces.contains(&"twig:block:base_header".to_string()));
        assert!(facts.iter().all(|fact| fact.role == FactRole::Definition));
        assert!(facts.iter().all(|fact| fact.evidence.snippet.is_none()));
        assert_eq!(
            facts
                .iter()
                .find(|fact| fact.surface.as_str() == "twig:block:base_header")
                .map(|fact| fact.kind),
            Some(SurfaceKind::TwigBlock)
        );
    }
}
