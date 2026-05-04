use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;

use crate::model::{Confidence, Evidence, Fact, FactRole, LineIndex};

pub fn extract(
    content: &str,
    relative_path: &Path,
    role: FactRole,
    include_snippets: bool,
) -> Vec<Fact> {
    let mut facts = FactCollector::new(content, relative_path, include_snippets);

    extract_component_calls(content, relative_path, role, include_snippets, &mut facts);
    extract_module_registers(content, relative_path, role, include_snippets, &mut facts);

    if role == FactRole::Definition {
        return facts.into_vec();
    }

    extract_repository_factory(content, relative_path, role, include_snippets, &mut facts);
    extract_shopware_services(content, relative_path, role, include_snippets, &mut facts);
    extract_shopware_state(content, relative_path, role, include_snippets, &mut facts);
    extract_route_literals(content, relative_path, role, include_snippets, &mut facts);
    extract_entity_literals(content, relative_path, role, include_snippets, &mut facts);
    extract_component_literals(content, relative_path, role, include_snippets, &mut facts);
    extract_embedded_twig_blocks(content, &mut facts);
    extract_snippet_calls(content, relative_path, role, include_snippets, &mut facts);

    facts.into_vec()
}

fn extract_embedded_twig_blocks(content: &str, facts: &mut FactCollector) {
    if !content.contains("{%") || !content.contains("block") {
        return;
    }

    for captures in embedded_twig_block_re().captures_iter(content) {
        let Some(block_match) = captures.get(1) else {
            continue;
        };

        facts.push(
            FactRole::Usage,
            format!("twig:block:{}", block_match.as_str()),
            facts.evidence(block_match.start()),
            Confidence::High,
            "twig.block.usage",
        );
    }
}

fn extract_component_calls(
    content: &str,
    _relative_path: &Path,
    role: FactRole,
    _include_snippets: bool,
    facts: &mut FactCollector,
) {
    for captures in component_call_re().captures_iter(content) {
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

        let method = method_match.as_str();
        if role == FactRole::Usage || method == "register" {
            facts.push(
                role,
                format!("admin:component:{}", component.value),
                facts.evidence(component.start),
                Confidence::High,
                format!("shopware.component.{method}"),
            );
        }

        if role == FactRole::Usage
            && method == "extend"
            && let Some(parent) = literals.get(1)
        {
            facts.push(
                role,
                format!("admin:component:{}", parent.value),
                facts.evidence(parent.start),
                Confidence::High,
                "shopware.component.extend.parent",
            );
        }
    }
}

fn extract_module_registers(
    content: &str,
    _relative_path: &Path,
    role: FactRole,
    _include_snippets: bool,
    facts: &mut FactCollector,
) {
    for call_match in module_register_re().find_iter(content) {
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
            facts.evidence(module.start),
            Confidence::High,
            "shopware.module.register",
        );

        let call_body = &content[open_paren + 1..close_paren];
        for routes_match in routes_object_re().find_iter(call_body) {
            let open_brace = open_paren + 1 + routes_match.end() - 1;
            for route_key in collect_top_level_object_keys(content, open_brace) {
                facts.push(
                    role,
                    format!("admin:route:{route_prefix}.{}", route_key.value),
                    facts.evidence(route_key.start),
                    Confidence::High,
                    "shopware.module.route",
                );
            }
        }
    }
}

fn extract_repository_factory(
    content: &str,
    _relative_path: &Path,
    role: FactRole,
    _include_snippets: bool,
    facts: &mut FactCollector,
) {
    for call_match in repository_factory_re().find_iter(content) {
        let literals = read_string_literals_in_call(content, call_match.end() - 1, 1);
        let Some(entity) = literals.first() else {
            continue;
        };

        if looks_like_entity_name(&entity.value) {
            facts.push(
                role,
                format!("dal:entity:{}", entity.value),
                facts.evidence(entity.start),
                Confidence::High,
                "repositoryFactory.create",
            );
        }
    }
}

fn extract_shopware_services(
    content: &str,
    _relative_path: &Path,
    role: FactRole,
    _include_snippets: bool,
    facts: &mut FactCollector,
) {
    for call_match in service_call_re().find_iter(content) {
        let literals = read_string_literals_in_call(content, call_match.end() - 1, 1);
        let Some(service) = literals.first() else {
            continue;
        };

        facts.push(
            role,
            format!("service:id:{}", service.value),
            facts.evidence(service.start),
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
    for call_match in state_call_re().find_iter(content) {
        let literals = read_string_literals_in_call(content, call_match.end() - 1, 1);
        let Some(store) = literals.first() else {
            continue;
        };
        emit_state_store(content, relative_path, role, include_snippets, facts, store);
    }

    for bracket_match in state_bracket_re().find_iter(content) {
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
    _content: &str,
    _relative_path: &Path,
    role: FactRole,
    _include_snippets: bool,
    facts: &mut FactCollector,
    store: &Literal,
) {
    facts.push(
        role,
        format!("admin:state-store:{}", store.value),
        facts.evidence(store.start),
        Confidence::High,
        "shopware.state",
    );
}

fn extract_route_literals(
    content: &str,
    _relative_path: &Path,
    role: FactRole,
    _include_snippets: bool,
    facts: &mut FactCollector,
) {
    extract_property_literals(content, route_property_re(), |literal, property| {
        if looks_like_admin_route_name(&literal.value) {
            facts.push(
                role,
                format!("admin:route:{}", literal.value),
                facts.evidence(literal.start),
                Confidence::High,
                format!("admin.route.{property}"),
            );
        }
    });

    extract_attribute_literals(content, route_attribute_re(), |literal, attribute| {
        if looks_like_admin_route_name(&literal.value) {
            facts.push(
                role,
                format!("admin:route:{}", literal.value),
                facts.evidence(literal.start),
                Confidence::Medium,
                format!("admin.route.attribute.{attribute}"),
            );
        }
    });
}

fn extract_entity_literals(
    content: &str,
    _relative_path: &Path,
    role: FactRole,
    _include_snippets: bool,
    facts: &mut FactCollector,
) {
    extract_property_literals(content, entity_property_re(), |literal, property| {
        if looks_like_entity_name(&literal.value) {
            facts.push(
                role,
                format!("dal:entity:{}", literal.value),
                facts.evidence(literal.start),
                Confidence::High,
                format!("admin.entity.{property}"),
            );
        }
    });

    extract_attribute_literals(content, entity_attribute_re(), |literal, attribute| {
        if looks_like_entity_name(&literal.value) {
            facts.push(
                role,
                format!("dal:entity:{}", literal.value),
                facts.evidence(literal.start),
                Confidence::High,
                format!("admin.entity.attribute.{attribute}"),
            );
        }
    });
}

fn extract_component_literals(
    content: &str,
    _relative_path: &Path,
    role: FactRole,
    _include_snippets: bool,
    facts: &mut FactCollector,
) {
    extract_property_literals(content, component_property_re(), |literal, property| {
        if looks_like_admin_component(&literal.value) {
            facts.push(
                role,
                format!("admin:component:{}", literal.value),
                facts.evidence(literal.start),
                Confidence::Medium,
                format!("admin.component.{property}"),
            );
        }
    });

    extract_attribute_literals(content, component_attribute_re(), |literal, attribute| {
        if looks_like_admin_component(&literal.value) {
            facts.push(
                role,
                format!("admin:component:{}", literal.value),
                facts.evidence(literal.start),
                Confidence::Medium,
                format!("admin.component.attribute.{attribute}"),
            );
        }
    });

    for captures in vue_component_tag_re().captures_iter(content) {
        let Some(component) = captures.get(1) else {
            continue;
        };

        facts.push(
            role,
            format!("admin:component:{}", component.as_str()),
            facts.evidence(component.start()),
            Confidence::Medium,
            "vue.template.component",
        );
    }
}

fn extract_snippet_calls(
    content: &str,
    _relative_path: &Path,
    role: FactRole,
    _include_snippets: bool,
    facts: &mut FactCollector,
) {
    for captures in snippet_call_re().captures_iter(content) {
        let Some(method) = captures.get(1) else {
            continue;
        };
        let Some(call_match) = captures.get(0) else {
            continue;
        };
        let literals = read_string_literals_in_call(content, call_match.end() - 1, 1);
        let Some(snippet) = literals.first() else {
            continue;
        };

        if !looks_like_snippet_key(&snippet.value) {
            continue;
        }

        facts.push(
            role,
            format!("snippet:key:{}", snippet.value),
            facts.evidence(snippet.start),
            Confidence::High,
            format!("vue.i18n.{}", method.as_str()),
        );
    }
}

fn extract_property_literals(content: &str, regex: &Regex, mut emit: impl FnMut(&Literal, &str)) {
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

fn extract_attribute_literals(content: &str, regex: &Regex, mut emit: impl FnMut(&Literal, &str)) {
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

fn component_call_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\bShopware\s*\.\s*Component\s*\.\s*(override|extend|register)\s*\(").unwrap()
    })
}

fn module_register_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\bShopware\s*\.\s*Module\s*\.\s*register\s*\(").unwrap())
}

fn routes_object_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\broutes\s*:\s*\{").unwrap())
}

fn repository_factory_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\b(?:this\s*\.\s*)?repositoryFactory\s*\.\s*create\s*\(").unwrap()
    })
}

fn service_call_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\bShopware\s*\.\s*Service\s*\(").unwrap())
}

fn state_call_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\bShopware\s*\.\s*State(?:\s*\.\s*[A-Za-z_$][A-Za-z0-9_$]*)?\s*\(").unwrap()
    })
}

fn state_bracket_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\bShopware\s*\.\s*State\s*\.\s*[A-Za-z_$][A-Za-z0-9_$]*\s*\[").unwrap()
    })
}

fn route_property_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b(name|route|routeName|parentPath)\s*:\s*").unwrap())
}

fn route_attribute_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[:@]?(name|route|parent-path|parentPath)\s*=\s*").unwrap())
}

fn entity_property_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\b(entity|entityName|sourceEntity|targetEntity)\s*:\s*").unwrap()
    })
}

fn entity_attribute_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[:@]?(entity|entity-name|entityName)\s*=\s*").unwrap())
}

fn component_property_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b(component)\s*:\s*").unwrap())
}

fn component_attribute_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[:@]?(component|is)\s*=\s*").unwrap())
}

fn vue_component_tag_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"</?\s*(sw-[a-z0-9][a-z0-9-]*)\b").unwrap())
}

fn embedded_twig_block_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\{%-?\s*block\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap())
}

fn snippet_call_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?:\bthis\s*\.\s*)?\$(t|te|tc)\s*\(").unwrap())
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

fn looks_like_snippet_key(value: &str) -> bool {
    !value.is_empty()
        && value.contains('.')
        && !value.starts_with('.')
        && !value.ends_with('.')
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

struct FactCollector<'a> {
    content: &'a str,
    relative_path: &'a Path,
    include_snippets: bool,
    line_index: LineIndex,
    facts: Vec<Fact>,
}

impl<'a> FactCollector<'a> {
    fn new(content: &'a str, relative_path: &'a Path, include_snippets: bool) -> Self {
        Self {
            content,
            relative_path,
            include_snippets,
            line_index: LineIndex::new(content),
            facts: Vec::new(),
        }
    }

    fn evidence(&self, byte_offset: usize) -> Evidence {
        let (line, column) = self
            .line_index
            .line_col(self.content, byte_offset.min(self.content.len()));

        Evidence {
            path: self.relative_path.to_path_buf(),
            line,
            column: Some(column),
            snippet: self
                .include_snippets
                .then(|| self.line_index.snippet(self.content, line))
                .flatten(),
        }
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
this.$t('sw-order.general.mainMenuItemGeneral');
this.$te('sw-order.general.mainMenuItemList');
const template = `{% block sw_product_detail_content_tabs_reviews %}
    {% parent %}
{% endblock %}`;
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
        assert!(surfaces.contains(&"snippet:key:sw-order.general.mainMenuItemGeneral".to_string()));
        assert!(surfaces.contains(&"snippet:key:sw-order.general.mainMenuItemList".to_string()));
        assert!(
            surfaces.contains(&"twig:block:sw_product_detail_content_tabs_reviews".to_string())
        );
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
    {{ $tc('sw-order.list.textOrdersTotal', 2) }}
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

        let facts = extract(content, Path::new("component.vue"), FactRole::Usage, false);
        let surfaces = surfaces(&facts);

        assert!(surfaces.contains(&"admin:component:sw-product-detail".to_string()));
        assert!(surfaces.contains(&"admin:component:sw-product-card".to_string()));
        assert!(surfaces.contains(&"admin:route:sw.product.detail".to_string()));
        assert!(surfaces.contains(&"dal:entity:product".to_string()));
        assert!(surfaces.contains(&"dal:entity:category".to_string()));
        assert!(surfaces.contains(&"admin:state-store:swCategoryDetail".to_string()));
        assert!(surfaces.contains(&"snippet:key:sw-order.list.textOrdersTotal".to_string()));
        assert!(facts.iter().all(|fact| fact.role == FactRole::Usage));
        assert!(facts.iter().all(|fact| fact.evidence.snippet.is_none()));
    }

    #[test]
    fn definition_extraction_only_emits_admin_extension_points() {
        let content = r#"
Shopware.Component.override('sw-product-detail', {});
Shopware.Component.extend('sw-product-card', 'sw-product-list', {});
Shopware.Component.register('sw-product-detail', {});
Shopware.Module.register('sw-product', {
    routes: {
        detail: { component: 'sw-product-detail', meta: { parentPath: 'sw.product.index' } }
    }
});
const repo = repositoryFactory.create('product');
Shopware.Service('acl');
Shopware.State.get('swProductDetail');
this.$router.push({ name: 'sw.product.detail' });
this.$t('sw-order.general.mainMenuItemGeneral');
"#;

        let facts = extract(
            content,
            Path::new("administration/src/main.js"),
            FactRole::Definition,
            false,
        );
        let surfaces = surfaces(&facts);

        assert!(surfaces.contains(&"admin:component:sw-product-detail".to_string()));
        assert!(surfaces.contains(&"admin:route:sw.product".to_string()));
        assert!(surfaces.contains(&"admin:route:sw.product.detail".to_string()));
        assert!(!surfaces.contains(&"admin:component:sw-product-card".to_string()));
        assert!(!surfaces.contains(&"admin:component:sw-product-list".to_string()));
        assert!(!surfaces.contains(&"dal:entity:product".to_string()));
        assert!(!surfaces.contains(&"service:id:acl".to_string()));
        assert!(!surfaces.contains(&"admin:state-store:swProductDetail".to_string()));
        assert!(
            !surfaces.contains(&"snippet:key:sw-order.general.mainMenuItemGeneral".to_string())
        );
        assert!(facts.iter().all(|fact| fact.role == FactRole::Definition));
    }
}
