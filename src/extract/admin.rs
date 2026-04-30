use std::path::Path;

use regex::Regex;

use crate::model::{Confidence, Evidence, Fact, FactRole, line_col_at, line_snippet};

pub fn extract(
    content: &str,
    relative_path: &Path,
    role: FactRole,
    include_snippets: bool,
) -> Vec<Fact> {
    let mut facts = FactCollector::new();

    extract_component_calls(content, relative_path, role, include_snippets, &mut facts);
    extract_module_registers(content, relative_path, role, include_snippets, &mut facts);
    extract_repository_factory(content, relative_path, role, include_snippets, &mut facts);
    extract_shopware_services(content, relative_path, role, include_snippets, &mut facts);
    extract_shopware_state(content, relative_path, role, include_snippets, &mut facts);
    extract_route_literals(content, relative_path, role, include_snippets, &mut facts);
    extract_entity_literals(content, relative_path, role, include_snippets, &mut facts);
    extract_component_literals(content, relative_path, role, include_snippets, &mut facts);

    facts.into_vec()
}

fn extract_component_calls(
    content: &str,
    relative_path: &Path,
    role: FactRole,
    include_snippets: bool,
    facts: &mut FactCollector,
) {
    let regex =
        Regex::new(r"\bShopware\s*\.\s*Component\s*\.\s*(override|extend|register)\s*\(").unwrap();

    for captures in regex.captures_iter(content) {
        let Some(method_match) = captures.get(1) else {
            continue;
        };
        let Some(call_match) = captures.get(0) else {
            continue;
        };

        let literals = read_string_literals_in_call(content, call_match.end() - 1, 2);
        let Some(component) = literals.first() else {
            continue;
        };

        facts.push(
            role,
            format!("admin:component:{}", component.value),
            evidence(content, relative_path, component.start, include_snippets),
            Confidence::High,
            format!("shopware.component.{}", method_match.as_str()),
        );

        if method_match.as_str() == "extend"
            && let Some(parent) = literals.get(1)
        {
            facts.push(
                role,
                format!("admin:component:{}", parent.value),
                evidence(content, relative_path, parent.start, include_snippets),
                Confidence::High,
                "shopware.component.extend.parent",
            );
        }
    }
}

fn extract_module_registers(
    content: &str,
    relative_path: &Path,
    role: FactRole,
    include_snippets: bool,
    facts: &mut FactCollector,
) {
    let regex = Regex::new(r"\bShopware\s*\.\s*Module\s*\.\s*register\s*\(").unwrap();
    let routes_regex = Regex::new(r"\broutes\s*:\s*\{").unwrap();

    for call_match in regex.find_iter(content) {
        let open_paren = call_match.end() - 1;
        let Some(close_paren) = find_matching_delimiter(content, open_paren, b'(', b')') else {
            continue;
        };
        let literals = read_string_literals_in_range(content, open_paren + 1, close_paren, 1);
        let Some(module) = literals.first() else {
            continue;
        };
        let route_prefix = module_route_prefix(&module.value);

        facts.push(
            role,
            format!("admin:route:{route_prefix}"),
            evidence(content, relative_path, module.start, include_snippets),
            Confidence::High,
            "shopware.module.register",
        );

        let call_body = &content[open_paren + 1..close_paren];
        for routes_match in routes_regex.find_iter(call_body) {
            let open_brace = open_paren + 1 + routes_match.end() - 1;
            for route_key in collect_top_level_object_keys(content, open_brace) {
                facts.push(
                    role,
                    format!("admin:route:{route_prefix}.{}", route_key.value),
                    evidence(content, relative_path, route_key.start, include_snippets),
                    Confidence::High,
                    "shopware.module.route",
                );
            }
        }
    }
}

fn extract_repository_factory(
    content: &str,
    relative_path: &Path,
    role: FactRole,
    include_snippets: bool,
    facts: &mut FactCollector,
) {
    let regex = Regex::new(r"\b(?:this\s*\.\s*)?repositoryFactory\s*\.\s*create\s*\(").unwrap();

    for call_match in regex.find_iter(content) {
        let literals = read_string_literals_in_call(content, call_match.end() - 1, 1);
        let Some(entity) = literals.first() else {
            continue;
        };

        if looks_like_entity_name(&entity.value) {
            facts.push(
                role,
                format!("dal:entity:{}", entity.value),
                evidence(content, relative_path, entity.start, include_snippets),
                Confidence::High,
                "repositoryFactory.create",
            );
        }
    }
}

fn extract_shopware_services(
    content: &str,
    relative_path: &Path,
    role: FactRole,
    include_snippets: bool,
    facts: &mut FactCollector,
) {
    let regex = Regex::new(r"\bShopware\s*\.\s*Service\s*\(").unwrap();

    for call_match in regex.find_iter(content) {
        let literals = read_string_literals_in_call(content, call_match.end() - 1, 1);
        let Some(service) = literals.first() else {
            continue;
        };

        facts.push(
            role,
            format!("service:id:{}", service.value),
            evidence(content, relative_path, service.start, include_snippets),
            Confidence::High,
            "shopware.service",
        );
    }
}

fn extract_shopware_state(
    content: &str,
    relative_path: &Path,
    role: FactRole,
    include_snippets: bool,
    facts: &mut FactCollector,
) {
    let call_regex =
        Regex::new(r"\bShopware\s*\.\s*State(?:\s*\.\s*[A-Za-z_$][A-Za-z0-9_$]*)?\s*\(").unwrap();
    let bracket_regex =
        Regex::new(r"\bShopware\s*\.\s*State\s*\.\s*[A-Za-z_$][A-Za-z0-9_$]*\s*\[").unwrap();

    for call_match in call_regex.find_iter(content) {
        let literals = read_string_literals_in_call(content, call_match.end() - 1, 1);
        let Some(store) = literals.first() else {
            continue;
        };
        emit_state_store(content, relative_path, role, include_snippets, facts, store);
    }

    for bracket_match in bracket_regex.find_iter(content) {
        let Some(store) = read_string_literal_at(content, bracket_match.end()) else {
            continue;
        };
        emit_state_store(
            content,
            relative_path,
            role,
            include_snippets,
            facts,
            &store,
        );
    }
}

fn emit_state_store(
    content: &str,
    relative_path: &Path,
    role: FactRole,
    include_snippets: bool,
    facts: &mut FactCollector,
    store: &Literal,
) {
    facts.push(
        role,
        format!("admin:state-store:{}", store.value),
        evidence(content, relative_path, store.start, include_snippets),
        Confidence::High,
        "shopware.state",
    );
}

fn extract_route_literals(
    content: &str,
    relative_path: &Path,
    role: FactRole,
    include_snippets: bool,
    facts: &mut FactCollector,
) {
    extract_property_literals(
        content,
        r"\b(name|route|routeName|parentPath)\s*:\s*",
        |literal, property| {
            if looks_like_admin_route_name(&literal.value) {
                facts.push(
                    role,
                    format!("admin:route:{}", literal.value),
                    evidence(content, relative_path, literal.start, include_snippets),
                    Confidence::High,
                    format!("admin.route.{property}"),
                );
            }
        },
    );

    extract_attribute_literals(
        content,
        r"[:@]?(name|route|parent-path|parentPath)\s*=\s*",
        |literal, attribute| {
            if looks_like_admin_route_name(&literal.value) {
                facts.push(
                    role,
                    format!("admin:route:{}", literal.value),
                    evidence(content, relative_path, literal.start, include_snippets),
                    Confidence::Medium,
                    format!("admin.route.attribute.{attribute}"),
                );
            }
        },
    );
}

fn extract_entity_literals(
    content: &str,
    relative_path: &Path,
    role: FactRole,
    include_snippets: bool,
    facts: &mut FactCollector,
) {
    extract_property_literals(
        content,
        r"\b(entity|entityName|sourceEntity|targetEntity)\s*:\s*",
        |literal, property| {
            if looks_like_entity_name(&literal.value) {
                facts.push(
                    role,
                    format!("dal:entity:{}", literal.value),
                    evidence(content, relative_path, literal.start, include_snippets),
                    Confidence::High,
                    format!("admin.entity.{property}"),
                );
            }
        },
    );

    extract_attribute_literals(
        content,
        r"[:@]?(entity|entity-name|entityName)\s*=\s*",
        |literal, attribute| {
            if looks_like_entity_name(&literal.value) {
                facts.push(
                    role,
                    format!("dal:entity:{}", literal.value),
                    evidence(content, relative_path, literal.start, include_snippets),
                    Confidence::High,
                    format!("admin.entity.attribute.{attribute}"),
                );
            }
        },
    );
}

fn extract_component_literals(
    content: &str,
    relative_path: &Path,
    role: FactRole,
    include_snippets: bool,
    facts: &mut FactCollector,
) {
    extract_property_literals(content, r"\b(component)\s*:\s*", |literal, property| {
        if looks_like_admin_component(&literal.value) {
            facts.push(
                role,
                format!("admin:component:{}", literal.value),
                evidence(content, relative_path, literal.start, include_snippets),
                Confidence::Medium,
                format!("admin.component.{property}"),
            );
        }
    });

    extract_attribute_literals(
        content,
        r"[:@]?(component|is)\s*=\s*",
        |literal, attribute| {
            if looks_like_admin_component(&literal.value) {
                facts.push(
                    role,
                    format!("admin:component:{}", literal.value),
                    evidence(content, relative_path, literal.start, include_snippets),
                    Confidence::Medium,
                    format!("admin.component.attribute.{attribute}"),
                );
            }
        },
    );

    let tag_regex = Regex::new(r"</?\s*(sw-[a-z0-9][a-z0-9-]*)\b").unwrap();
    for captures in tag_regex.captures_iter(content) {
        let Some(component) = captures.get(1) else {
            continue;
        };

        facts.push(
            role,
            format!("admin:component:{}", component.as_str()),
            evidence(content, relative_path, component.start(), include_snippets),
            Confidence::Medium,
            "vue.template.component",
        );
    }
}

fn extract_property_literals(content: &str, pattern: &str, mut emit: impl FnMut(&Literal, &str)) {
    let regex = Regex::new(pattern).unwrap();

    for captures in regex.captures_iter(content) {
        let Some(property) = captures.get(1) else {
            continue;
        };
        let Some(pattern_match) = captures.get(0) else {
            continue;
        };
        let Some(literal) = read_string_literal_at(content, pattern_match.end()) else {
            continue;
        };

        emit(&literal, property.as_str());
    }
}

fn extract_attribute_literals(content: &str, pattern: &str, mut emit: impl FnMut(&Literal, &str)) {
    let regex = Regex::new(pattern).unwrap();

    for captures in regex.captures_iter(content) {
        let Some(attribute) = captures.get(1) else {
            continue;
        };
        let Some(pattern_match) = captures.get(0) else {
            continue;
        };
        let Some(literal) = read_string_literal_at(content, pattern_match.end()) else {
            continue;
        };

        emit(&literal, attribute.as_str());
    }
}

#[derive(Debug, Clone)]
struct Literal {
    value: String,
    start: usize,
    end: usize,
}

fn read_string_literals_in_call(content: &str, open_paren: usize, max: usize) -> Vec<Literal> {
    let Some(close_paren) = find_matching_delimiter(content, open_paren, b'(', b')') else {
        return Vec::new();
    };

    read_string_literals_in_range(content, open_paren + 1, close_paren, max)
}

fn read_string_literals_in_range(
    content: &str,
    start: usize,
    end: usize,
    max: usize,
) -> Vec<Literal> {
    let mut literals = Vec::new();
    let bytes = content.as_bytes();
    let mut index = start.min(content.len());
    let end = end.min(content.len());

    while index < end && literals.len() < max {
        if let Some(next_index) = skip_comment(content, index) {
            index = next_index;
            continue;
        }

        if matches!(bytes[index], b'\'' | b'"' | b'`')
            && let Some(literal) = read_string_literal_at(content, index)
        {
            index = literal.end;
            literals.push(literal);
            continue;
        }

        index += 1;
    }

    literals
}

fn read_string_literal_at(content: &str, offset: usize) -> Option<Literal> {
    let bytes = content.as_bytes();
    let mut index = skip_whitespace_and_comments(content, offset.min(content.len()));

    if index >= content.len() {
        return None;
    }

    let quote = bytes[index];
    if !matches!(quote, b'\'' | b'"' | b'`') {
        return None;
    }

    index += 1;
    let value_start = index;
    let mut escaped = false;

    while index < content.len() {
        let byte = bytes[index];

        if escaped {
            escaped = false;
            index += 1;
            continue;
        }

        if byte == b'\\' {
            escaped = true;
            index += 1;
            continue;
        }

        if quote == b'`' && byte == b'$' && bytes.get(index + 1) == Some(&b'{') {
            return None;
        }

        if byte == quote {
            let raw = &content[value_start..index];
            return Some(Literal {
                value: unescape_js_literal(raw),
                start: value_start,
                end: index + 1,
            });
        }

        index += 1;
    }

    None
}

fn unescape_js_literal(raw: &str) -> String {
    let mut result = String::with_capacity(raw.len());
    let mut chars = raw.chars();

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            result.push(ch);
            continue;
        }

        match chars.next() {
            Some('n') => result.push('\n'),
            Some('r') => result.push('\r'),
            Some('t') => result.push('\t'),
            Some('"') => result.push('"'),
            Some('\'') => result.push('\''),
            Some('`') => result.push('`'),
            Some('\\') => result.push('\\'),
            Some(other) => result.push(other),
            None => result.push('\\'),
        }
    }

    result
}

fn find_matching_delimiter(content: &str, open_index: usize, open: u8, close: u8) -> Option<usize> {
    let bytes = content.as_bytes();
    if bytes.get(open_index) != Some(&open) {
        return None;
    }

    let mut depth = 1usize;
    let mut index = open_index + 1;

    while index < content.len() {
        if let Some(next_index) = skip_comment(content, index) {
            index = next_index;
            continue;
        }

        match bytes[index] {
            b'\'' | b'"' | b'`' => {
                index = skip_quoted_string(content, index)?;
            }
            byte if byte == open => {
                depth += 1;
                index += 1;
            }
            byte if byte == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
                index += 1;
            }
            _ => index += 1,
        }
    }

    None
}

fn skip_quoted_string(content: &str, start: usize) -> Option<usize> {
    let bytes = content.as_bytes();
    let quote = *bytes.get(start)?;
    let mut escaped = false;
    let mut index = start + 1;

    while index < content.len() {
        let byte = bytes[index];

        if escaped {
            escaped = false;
            index += 1;
            continue;
        }

        if byte == b'\\' {
            escaped = true;
            index += 1;
            continue;
        }

        if byte == quote {
            return Some(index + 1);
        }

        index += 1;
    }

    None
}

fn skip_comment(content: &str, start: usize) -> Option<usize> {
    let bytes = content.as_bytes();

    if bytes.get(start) == Some(&b'/') && bytes.get(start + 1) == Some(&b'/') {
        return content[start + 2..]
            .find('\n')
            .map(|offset| start + 2 + offset + 1)
            .or(Some(content.len()));
    }

    if bytes.get(start) == Some(&b'/') && bytes.get(start + 1) == Some(&b'*') {
        return content[start + 2..]
            .find("*/")
            .map(|offset| start + 2 + offset + 2)
            .or(Some(content.len()));
    }

    None
}

fn skip_whitespace_and_comments(content: &str, mut index: usize) -> usize {
    loop {
        while content
            .as_bytes()
            .get(index)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            index += 1;
        }

        if let Some(next_index) = skip_comment(content, index) {
            index = next_index;
        } else {
            return index;
        }
    }
}

fn collect_top_level_object_keys(content: &str, open_brace: usize) -> Vec<Literal> {
    let Some(close_brace) = find_matching_delimiter(content, open_brace, b'{', b'}') else {
        return Vec::new();
    };

    let bytes = content.as_bytes();
    let mut keys = Vec::new();
    let mut index = open_brace + 1;

    while index < close_brace {
        index = skip_whitespace_and_comments(content, index);

        while bytes.get(index).is_some_and(|byte| *byte == b',') {
            index += 1;
            index = skip_whitespace_and_comments(content, index);
        }

        let key = if bytes
            .get(index)
            .is_some_and(|byte| matches!(*byte, b'\'' | b'"' | b'`'))
        {
            let Some(literal) = read_string_literal_at(content, index) else {
                break;
            };
            index = literal.end;
            literal
        } else {
            let key_start = index;
            while bytes.get(index).is_some_and(|byte| {
                byte.is_ascii_alphanumeric() || matches!(*byte, b'_' | b'$' | b'-')
            }) {
                index += 1;
            }

            if key_start == index {
                index += 1;
                continue;
            }

            Literal {
                value: content[key_start..index].to_string(),
                start: key_start,
                end: index,
            }
        };

        index = skip_whitespace_and_comments(content, index);
        if bytes.get(index) == Some(&b':') {
            keys.push(key);
            index += 1;
            index = skip_top_level_value(content, index, close_brace);
        }
    }

    keys
}

fn skip_top_level_value(content: &str, start: usize, object_end: usize) -> usize {
    let bytes = content.as_bytes();
    let mut index = start;
    let mut paren_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut bracket_depth = 0usize;

    while index < object_end {
        if let Some(next_index) = skip_comment(content, index) {
            index = next_index;
            continue;
        }

        match bytes[index] {
            b'\'' | b'"' | b'`' => {
                if let Some(next_index) = skip_quoted_string(content, index) {
                    index = next_index;
                } else {
                    return object_end;
                }
            }
            b'(' => {
                paren_depth += 1;
                index += 1;
            }
            b')' => {
                paren_depth = paren_depth.saturating_sub(1);
                index += 1;
            }
            b'{' => {
                brace_depth += 1;
                index += 1;
            }
            b'}' => {
                if brace_depth == 0 {
                    return index;
                }
                brace_depth -= 1;
                index += 1;
            }
            b'[' => {
                bracket_depth += 1;
                index += 1;
            }
            b']' => {
                bracket_depth = bracket_depth.saturating_sub(1);
                index += 1;
            }
            b',' if paren_depth == 0 && brace_depth == 0 && bracket_depth == 0 => {
                return index + 1;
            }
            _ => index += 1,
        }
    }

    object_end
}

fn module_route_prefix(module: &str) -> String {
    if module.contains('.') {
        module.to_string()
    } else {
        module.replace('-', ".")
    }
}

fn looks_like_admin_component(value: &str) -> bool {
    value.starts_with("sw-") || value.starts_with("mt-")
}

fn looks_like_admin_route_name(value: &str) -> bool {
    value.starts_with("sw.")
        && value.contains('.')
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}

fn looks_like_entity_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '_' | '-'))
        && value.chars().any(|ch| ch.is_ascii_lowercase())
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
    use crate::model::SurfaceKind;

    fn surfaces(facts: &[Fact]) -> Vec<String> {
        facts
            .iter()
            .map(|fact| fact.surface.as_str().to_string())
            .collect()
    }

    #[test]
    fn extracts_shopware_admin_literals() {
        let content = r#"
Shopware.Component.override('sw-product-detail', {});
Shopware.Component.extend('sw-product-card', 'sw-product-list', {});
Shopware.Component.register('sw-custom-card', { template: '<sw-card />' });
Shopware.Module.register('sw-product', {
    routes: {
        index: { component: 'sw-product-list' },
        detail: { component: 'sw-product-detail', meta: { parentPath: 'sw.product.index' } }
    }
});
const repo = repositoryFactory.create('product');
Shopware.Service('acl');
Shopware.State.get('swProductDetail');
this.$router.push({ name: 'sw.product.detail' });
"#;

        let facts = extract(
            content,
            Path::new("administration/src/main.js"),
            FactRole::Usage,
            true,
        );
        let surfaces = surfaces(&facts);

        assert!(surfaces.contains(&"admin:component:sw-product-detail".to_string()));
        assert!(surfaces.contains(&"admin:component:sw-product-card".to_string()));
        assert!(surfaces.contains(&"admin:component:sw-product-list".to_string()));
        assert!(surfaces.contains(&"admin:component:sw-custom-card".to_string()));
        assert!(surfaces.contains(&"admin:component:sw-card".to_string()));
        assert!(surfaces.contains(&"admin:route:sw.product".to_string()));
        assert!(surfaces.contains(&"admin:route:sw.product.index".to_string()));
        assert!(surfaces.contains(&"admin:route:sw.product.detail".to_string()));
        assert!(surfaces.contains(&"dal:entity:product".to_string()));
        assert!(surfaces.contains(&"service:id:acl".to_string()));
        assert!(surfaces.contains(&"admin:state-store:swProductDetail".to_string()));
        assert!(facts.iter().any(|fact| fact.evidence.snippet.is_some()));
        assert_eq!(
            facts
                .iter()
                .find(|fact| fact.surface.as_str() == "admin:route:sw.product.detail")
                .map(|fact| fact.kind),
            Some(SurfaceKind::AdminRoute)
        );
    }

    #[test]
    fn extracts_vue_template_and_script_literals() {
        let content = r#"
<template>
    <sw-product-detail entity="product" route="sw.product.detail" component="sw-product-card" />
</template>
<script>
export default {
    created() {
        this.repositoryFactory.create(`category`);
        Shopware.State.getters['swCategoryDetail'];
    }
}
</script>
"#;

        let facts = extract(
            content,
            Path::new("component.vue"),
            FactRole::Definition,
            false,
        );
        let surfaces = surfaces(&facts);

        assert!(surfaces.contains(&"admin:component:sw-product-detail".to_string()));
        assert!(surfaces.contains(&"admin:component:sw-product-card".to_string()));
        assert!(surfaces.contains(&"admin:route:sw.product.detail".to_string()));
        assert!(surfaces.contains(&"dal:entity:product".to_string()));
        assert!(surfaces.contains(&"dal:entity:category".to_string()));
        assert!(surfaces.contains(&"admin:state-store:swCategoryDetail".to_string()));
        assert!(facts.iter().all(|fact| fact.role == FactRole::Definition));
        assert!(facts.iter().all(|fact| fact.evidence.snippet.is_none()));
    }
}
