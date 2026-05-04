use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::OnceLock;

use anyhow::Result;
use regex::Regex;

use crate::extract::RelatedFileResolver;
use crate::model::{Confidence, Evidence, Fact, FactRole, LineIndex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentRegisterImport {
    pub component: String,
    pub import: String,
}

pub fn extract(
    content: &str,
    relative_path: &Path,
    role: FactRole,
    include_snippets: bool,
) -> Vec<Fact> {
    let mut related_files = |_: &Path, _: &str| Ok(None);
    extract_with_related(
        content,
        relative_path,
        role,
        include_snippets,
        &mut related_files,
    )
    .unwrap_or_default()
}

pub fn extract_with_related(
    content: &str,
    relative_path: &Path,
    role: FactRole,
    include_snippets: bool,
    related_files: &mut RelatedFileResolver<'_>,
) -> Result<Vec<Fact>> {
    let mut facts = FactCollector::new(content, relative_path, include_snippets);

    extract_component_calls(
        content,
        relative_path,
        role,
        include_snippets,
        related_files,
        &mut facts,
    )?;
    extract_module_registers(content, relative_path, role, include_snippets, &mut facts);

    if role == FactRole::Definition {
        return Ok(facts.into_vec());
    }

    extract_repository_factory(content, relative_path, role, include_snippets, &mut facts);
    extract_shopware_services(content, relative_path, role, include_snippets, &mut facts);
    extract_shopware_state(content, relative_path, role, include_snippets, &mut facts);
    extract_route_literals(content, relative_path, role, include_snippets, &mut facts);
    extract_entity_literals(content, relative_path, role, include_snippets, &mut facts);
    extract_component_literals(content, relative_path, role, include_snippets, &mut facts);
    extract_embedded_twig_blocks(content, &mut facts);
    extract_snippet_calls(content, relative_path, role, include_snippets, &mut facts);

    Ok(facts.into_vec())
}

pub fn extract_component_option_definitions(
    content: &str,
    relative_path: &Path,
    component: &str,
    include_snippets: bool,
) -> Vec<Fact> {
    let mut facts = FactCollector::new(content, relative_path, include_snippets);

    for export_match in export_default_object_re().find_iter(content) {
        let options_open = export_match.end() - 1;
        emit_component_option_member_definitions(content, component, options_open, &mut facts);
    }

    facts.into_vec()
}

pub fn component_register_imports(content: &str) -> Vec<ComponentRegisterImport> {
    let mut imports = Vec::new();

    for captures in component_call_re().captures_iter(content) {
        let Some(method_match) = captures.get(1) else {
            continue;
        };
        if method_match.as_str() != "register" {
            continue;
        }
        let Some(call_match) = captures.get(0) else {
            continue;
        };

        let args = collect_top_level_arguments(content, call_match.end() - 1);
        let Some(component) = args
            .first()
            .and_then(|arg| read_string_literal_in_span(content, arg))
        else {
            continue;
        };
        let Some(import) = args
            .get(1)
            .and_then(|arg| read_dynamic_import_arg(content, arg))
        else {
            continue;
        };

        imports.push(ComponentRegisterImport {
            component: component.value,
            import: import.value,
        });
    }

    imports
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
    relative_path: &Path,
    role: FactRole,
    include_snippets: bool,
    related_files: &mut RelatedFileResolver<'_>,
    facts: &mut FactCollector,
) -> Result<()> {
    let template_imports = collect_template_imports(content);

    for captures in component_call_re().captures_iter(content) {
        let Some(method_match) = captures.get(1) else {
            continue;
        };
        let Some(call_match) = captures.get(0) else {
            continue;
        };

        let open_paren = call_match.end() - 1;
        let args = collect_top_level_arguments(content, open_paren);
        let Some(component) = args
            .first()
            .and_then(|arg| read_string_literal_in_span(content, arg))
        else {
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
            && let Some(parent) = args
                .get(1)
                .and_then(|arg| read_string_literal_in_span(content, arg))
        {
            facts.push(
                role,
                format!("admin:component:{}", parent.value),
                facts.evidence(parent.start),
                Confidence::High,
                "shopware.component.extend.parent",
            );
        }

        match (role, method) {
            (FactRole::Definition, "register") => {
                if let Some(options_open) =
                    args.get(1).and_then(|arg| object_arg_open(content, arg))
                {
                    emit_component_option_member_definitions(
                        content,
                        &component.value,
                        options_open,
                        facts,
                    );
                }
            }
            (FactRole::Usage, "override") => {
                if let Some(options_open) =
                    args.get(1).and_then(|arg| object_arg_open(content, arg))
                {
                    emit_component_extension_usages(
                        content,
                        relative_path,
                        &component.value,
                        options_open,
                        &template_imports,
                        include_snippets,
                        related_files,
                        facts,
                    )?;
                }
            }
            (FactRole::Usage, "extend") => {
                let Some(parent) = args
                    .get(1)
                    .and_then(|arg| read_string_literal_in_span(content, arg))
                else {
                    continue;
                };
                if let Some(options_open) =
                    args.get(2).and_then(|arg| object_arg_open(content, arg))
                {
                    emit_component_extension_usages(
                        content,
                        relative_path,
                        &parent.value,
                        options_open,
                        &template_imports,
                        include_snippets,
                        related_files,
                        facts,
                    )?;
                }
            }
            _ => {}
        }
    }

    Ok(())
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

fn emit_component_option_member_definitions(
    content: &str,
    component: &str,
    options_open: usize,
    facts: &mut FactCollector,
) {
    for property in collect_top_level_object_properties(content, options_open) {
        emit_option_member_property_definitions(content, component, &property, facts);
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_component_extension_usages(
    content: &str,
    relative_path: &Path,
    parent: &str,
    options_open: usize,
    template_imports: &HashMap<String, Literal>,
    include_snippets: bool,
    related_files: &mut RelatedFileResolver<'_>,
    facts: &mut FactCollector,
) -> Result<()> {
    let Some(options_close) = find_matching_delimiter(content, options_open, b'{', b'}') else {
        return Ok(());
    };

    let option_properties = collect_top_level_object_properties(content, options_open);
    for property in &option_properties {
        emit_option_member_property_usages(content, parent, property, facts);
    }

    emit_js_this_member_usages(content, options_open + 1, options_close, parent, facts);
    emit_component_template_usages(
        content,
        relative_path,
        parent,
        &option_properties,
        template_imports,
        include_snippets,
        related_files,
        facts,
    )?;

    Ok(())
}

fn emit_option_member_property_definitions(
    content: &str,
    component: &str,
    property: &ObjectProperty,
    facts: &mut FactCollector,
) {
    match property.key.value.as_str() {
        "data" => {
            for data_property in collect_data_return_properties(content, property) {
                emit_component_property_fact(
                    facts,
                    FactRole::Definition,
                    component,
                    &data_property,
                    "shopware.component.data.definition",
                );
            }
        }
        "computed" => {
            for computed_property in collect_member_object_properties(content, property) {
                emit_component_property_fact(
                    facts,
                    FactRole::Definition,
                    component,
                    &computed_property.key,
                    "shopware.component.computed.definition",
                );
            }
        }
        "methods" => {
            for method_property in collect_member_object_properties(content, property) {
                emit_component_method_fact(
                    facts,
                    FactRole::Definition,
                    component,
                    &method_property.key,
                    "shopware.component.method.definition",
                );
            }
        }
        _ => {}
    }
}

fn emit_option_member_property_usages(
    content: &str,
    parent: &str,
    property: &ObjectProperty,
    facts: &mut FactCollector,
) {
    match property.key.value.as_str() {
        "data" => {
            for data_property in collect_data_return_properties(content, property) {
                emit_component_property_fact(
                    facts,
                    FactRole::Usage,
                    parent,
                    &data_property,
                    "shopware.component.data.override",
                );
            }
        }
        "computed" => {
            for computed_property in collect_member_object_properties(content, property) {
                emit_component_property_fact(
                    facts,
                    FactRole::Usage,
                    parent,
                    &computed_property.key,
                    "shopware.component.computed.override",
                );
            }
        }
        "methods" => {
            for method_property in collect_member_object_properties(content, property) {
                emit_component_method_fact(
                    facts,
                    FactRole::Usage,
                    parent,
                    &method_property.key,
                    "shopware.component.method.override",
                );
            }
        }
        _ => {}
    }
}

fn collect_member_object_properties(
    content: &str,
    property: &ObjectProperty,
) -> Vec<ObjectProperty> {
    let value_start = skip_whitespace_and_comments(content, property.value_start);
    if content.as_bytes().get(value_start) != Some(&b'{') {
        return Vec::new();
    }

    collect_top_level_object_properties(content, value_start)
}

fn collect_data_return_properties(content: &str, property: &ObjectProperty) -> Vec<Literal> {
    let mut properties = Vec::new();
    let mut index = property.value_start;
    let end = property.value_end.min(content.len());

    while index < end {
        if let Some(next_index) = skip_comment(content, index) {
            index = next_index;
            continue;
        }

        if matches!(content.as_bytes().get(index), Some(b'\'' | b'"' | b'`')) {
            if let Some(next_index) = skip_quoted_string(content, index) {
                index = next_index;
                continue;
            }
            break;
        }

        if content
            .as_bytes()
            .get(index)
            .is_some_and(|byte| byte.is_ascii_alphabetic() || matches!(*byte, b'_' | b'$'))
        {
            let identifier_start = index;
            index += 1;
            while content
                .as_bytes()
                .get(index)
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'_' | b'$'))
            {
                index += 1;
            }

            if &content[identifier_start..index] == "return" {
                let return_value_start = skip_whitespace_and_comments(content, index);
                if let Some(open_brace) = returned_object_open(content, return_value_start, end) {
                    properties.extend(
                        collect_top_level_object_properties(content, open_brace)
                            .into_iter()
                            .map(|property| property.key),
                    );
                    break;
                }
            }
            continue;
        }

        if content.as_bytes().get(index) == Some(&b'=')
            && content.as_bytes().get(index + 1) == Some(&b'>')
        {
            let arrow_value_start = skip_whitespace_and_comments(content, index + 2);
            if let Some(open_brace) = returned_object_open(content, arrow_value_start, end) {
                properties.extend(
                    collect_top_level_object_properties(content, open_brace)
                        .into_iter()
                        .map(|property| property.key),
                );
                break;
            }
        }

        index += 1;
    }

    properties
}

fn returned_object_open(content: &str, start: usize, end: usize) -> Option<usize> {
    let bytes = content.as_bytes();
    let start = skip_whitespace_and_comments(content, start).min(end);

    if bytes.get(start) == Some(&b'{') {
        return Some(start);
    }

    if bytes.get(start) == Some(&b'(') {
        let inner_start = skip_whitespace_and_comments(content, start + 1).min(end);
        if bytes.get(inner_start) == Some(&b'{') {
            return Some(inner_start);
        }
    }

    None
}

fn emit_component_property_fact(
    facts: &mut FactCollector,
    role: FactRole,
    component: &str,
    property: &Literal,
    usage_kind: &'static str,
) {
    if !looks_like_component_member_name(&property.value) {
        return;
    }

    facts.push(
        role,
        format!("admin:component:property:{component}:{}", property.value),
        facts.evidence(property.start),
        Confidence::High,
        usage_kind,
    );
}

fn emit_component_method_fact(
    facts: &mut FactCollector,
    role: FactRole,
    component: &str,
    method: &Literal,
    usage_kind: &'static str,
) {
    if !looks_like_component_member_name(&method.value) {
        return;
    }

    facts.push(
        role,
        format!("admin:component:method:{component}:{}", method.value),
        facts.evidence(method.start),
        Confidence::High,
        usage_kind,
    );
}

#[allow(clippy::too_many_arguments)]
fn emit_component_template_usages(
    content: &str,
    relative_path: &Path,
    parent: &str,
    option_properties: &[ObjectProperty],
    template_imports: &HashMap<String, Literal>,
    include_snippets: bool,
    related_files: &mut RelatedFileResolver<'_>,
    facts: &mut FactCollector,
) -> Result<()> {
    for property in option_properties
        .iter()
        .filter(|property| property.key.value == "template")
    {
        if let Some(template_literal) = read_string_literal_in_value(content, property) {
            let start = template_literal.start;
            let end = template_literal.end.saturating_sub(1);
            emit_vue_template_member_usages(content, start, end, parent, facts);
            continue;
        }

        let template_variable = if property.shorthand {
            property.key.value.clone()
        } else {
            read_identifier_at(content, property.value_start)
                .map(|identifier| identifier.value)
                .unwrap_or_default()
        };

        let Some(import) = template_imports.get(&template_variable) else {
            continue;
        };
        let Some((template_path, template_content)) = related_files(relative_path, &import.value)?
        else {
            continue;
        };

        let mut template_facts =
            FactCollector::new(&template_content, &template_path, include_snippets);
        emit_vue_template_member_usages(
            &template_content,
            0,
            template_content.len(),
            parent,
            &mut template_facts,
        );
        facts.facts.extend(template_facts.into_vec());
    }

    Ok(())
}

fn read_string_literal_in_value(content: &str, property: &ObjectProperty) -> Option<Literal> {
    let literal = read_string_literal_at(content, property.value_start)?;
    if literal.end <= property.value_end {
        Some(literal)
    } else {
        None
    }
}

fn emit_js_this_member_usages(
    content: &str,
    start: usize,
    end: usize,
    parent: &str,
    facts: &mut FactCollector,
) {
    let bytes = content.as_bytes();
    let mut index = start.min(content.len());
    let end = end.min(content.len());

    while index < end {
        if let Some(next_index) = skip_comment(content, index) {
            index = next_index;
            continue;
        }

        if matches!(bytes[index], b'\'' | b'"' | b'`') {
            if let Some(next_index) = skip_quoted_string(content, index) {
                index = next_index;
                continue;
            }
            break;
        }

        if !matches_identifier_at(content, index, "this") {
            index += 1;
            continue;
        }

        let mut cursor = skip_whitespace_and_comments(content, index + 4);
        if bytes.get(cursor) != Some(&b'.') {
            index += 4;
            continue;
        }
        cursor = skip_whitespace_and_comments(content, cursor + 1);

        let Some(member) = read_identifier_at(content, cursor) else {
            index += 4;
            continue;
        };
        let next = skip_whitespace_and_comments(content, member.end);

        if member.value == "$super" {
            if bytes.get(next) == Some(&b'(') {
                for method in read_string_literals_in_call(content, next, 1) {
                    emit_component_method_fact(
                        facts,
                        FactRole::Usage,
                        parent,
                        &method,
                        "shopware.component.super-call",
                    );
                }
            }
            index = member.end;
            continue;
        }

        if member.value.starts_with('$') {
            index = member.end;
            continue;
        }

        if bytes.get(next) == Some(&b'(') {
            emit_component_method_fact(
                facts,
                FactRole::Usage,
                parent,
                &member,
                "shopware.component.this-method",
            );
        } else {
            emit_component_property_fact(
                facts,
                FactRole::Usage,
                parent,
                &member,
                "shopware.component.this-property",
            );
        }

        index = member.end;
    }
}

fn emit_vue_template_member_usages(
    content: &str,
    start: usize,
    end: usize,
    parent: &str,
    facts: &mut FactCollector,
) {
    let locals = collect_vue_template_locals(content, start, end);

    for captures in moustache_re().captures_iter(&content[start..end]) {
        let Some(expression) = captures.get(1) else {
            continue;
        };
        emit_vue_expression_member_usages(
            content,
            start + expression.start(),
            start + expression.end(),
            parent,
            &locals,
            false,
            facts,
        );
    }

    for captures in vue_bound_attribute_re().captures_iter(&content[start..end]) {
        let Some(attribute) = captures.get(1) else {
            continue;
        };
        let Some(value) = captures.get(3).or_else(|| captures.get(4)) else {
            continue;
        };
        let value_start = start + value.start();
        let value_end = start + value.end();
        let attribute = attribute.as_str();

        if attribute == "v-for" {
            if let Some(for_expression) = split_v_for_expression(&content[value_start..value_end]) {
                emit_vue_expression_member_usages(
                    content,
                    value_start + for_expression.iterable_start,
                    value_start + for_expression.iterable_end,
                    parent,
                    &locals,
                    false,
                    facts,
                );
            }
            continue;
        }

        let is_event = attribute.starts_with('@') || attribute.starts_with("v-on:");
        emit_vue_expression_member_usages(
            content,
            value_start,
            value_end,
            parent,
            &locals,
            is_event,
            facts,
        );
    }
}

fn collect_vue_template_locals(content: &str, start: usize, end: usize) -> HashSet<String> {
    let mut locals = HashSet::new();

    for captures in vue_bound_attribute_re().captures_iter(&content[start..end]) {
        let Some(attribute) = captures.get(1) else {
            continue;
        };
        if attribute.as_str() != "v-for" {
            continue;
        }

        let Some(value) = captures.get(3).or_else(|| captures.get(4)) else {
            continue;
        };
        if let Some(for_expression) = split_v_for_expression(value.as_str()) {
            locals.extend(for_expression.aliases);
        }
    }

    locals
}

fn emit_vue_expression_member_usages(
    content: &str,
    start: usize,
    end: usize,
    parent: &str,
    locals: &HashSet<String>,
    event_expression: bool,
    facts: &mut FactCollector,
) {
    if event_expression
        && let Some(identifier) = simple_identifier_expression(content, start, end)
        && !locals.contains(&identifier.value)
        && !is_reserved_template_identifier(&identifier.value)
    {
        emit_component_method_fact(
            facts,
            FactRole::Usage,
            parent,
            &identifier,
            "vue.template.method",
        );
        return;
    }

    let bytes = content.as_bytes();
    let mut index = start.min(content.len());
    let end = end.min(content.len());

    while index < end {
        if let Some(next_index) = skip_comment(content, index) {
            index = next_index;
            continue;
        }

        if matches!(bytes[index], b'\'' | b'"' | b'`') {
            if let Some(next_index) = skip_quoted_string(content, index) {
                index = next_index;
                continue;
            }
            break;
        }

        let Some(identifier) = read_identifier_at(content, index) else {
            index += 1;
            continue;
        };
        index = identifier.end;

        if locals.contains(&identifier.value) || is_reserved_template_identifier(&identifier.value)
        {
            continue;
        }

        if previous_non_whitespace_byte(content, identifier.start, start) == Some(b'.') {
            continue;
        }

        let next = skip_whitespace_and_comments(content, identifier.end);
        if bytes.get(next) == Some(&b'(') {
            emit_component_method_fact(
                facts,
                FactRole::Usage,
                parent,
                &identifier,
                "vue.template.method",
            );
        } else if bytes.get(next) != Some(&b':') {
            emit_component_property_fact(
                facts,
                FactRole::Usage,
                parent,
                &identifier,
                "vue.template.property",
            );
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

fn export_default_object_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\bexport\s+default\s*\{").unwrap())
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

fn template_import_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\bimport\s+([A-Za-z_$][A-Za-z0-9_$]*)\s+from\s*").unwrap())
}

fn template_require_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\b(?:const|let|var)\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*=\s*require\s*\(").unwrap()
    })
}

fn moustache_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?s)\{\{(.*?)\}\}").unwrap())
}

fn vue_bound_attribute_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"(?s)(v-if|v-else-if|v-show|v-for|v-bind(?::[A-Za-z0-9_:$.-]+)?|v-on(?::[A-Za-z0-9_:$.-]+)?|:[A-Za-z0-9_:$.-]+|@[A-Za-z0-9_:$.-]+)\s*=\s*("([^"]*)"|'([^']*)')"#,
        )
        .unwrap()
    })
}

#[derive(Debug, Clone)]
struct ArgSpan {
    start: usize,
    end: usize,
}

#[derive(Debug, Clone)]
struct Literal {
    value: String,
    start: usize,
    end: usize,
}

#[derive(Debug, Clone)]
struct ObjectProperty {
    key: Literal,
    value_start: usize,
    value_end: usize,
    shorthand: bool,
}

#[derive(Debug)]
struct ForExpression {
    aliases: Vec<String>,
    iterable_start: usize,
    iterable_end: usize,
}

fn collect_template_imports(content: &str) -> HashMap<String, Literal> {
    let mut imports = HashMap::new();

    for captures in template_import_re().captures_iter(content) {
        let Some(variable) = captures.get(1) else {
            continue;
        };
        let Some(import_match) = captures.get(0) else {
            continue;
        };
        let Some(path) = read_string_literal_at(content, import_match.end()) else {
            continue;
        };
        if looks_like_twig_template_import(&path.value) {
            imports.insert(variable.as_str().to_owned(), path);
        }
    }

    for captures in template_require_re().captures_iter(content) {
        let Some(variable) = captures.get(1) else {
            continue;
        };
        let Some(require_match) = captures.get(0) else {
            continue;
        };
        let literals = read_string_literals_in_call(content, require_match.end() - 1, 1);
        let Some(path) = literals.into_iter().next() else {
            continue;
        };
        if looks_like_twig_template_import(&path.value) {
            imports.insert(variable.as_str().to_owned(), path);
        }
    }

    imports
}

fn collect_top_level_arguments(content: &str, open_paren: usize) -> Vec<ArgSpan> {
    let Some(close_paren) = find_matching_delimiter(content, open_paren, b'(', b')') else {
        return Vec::new();
    };

    let bytes = content.as_bytes();
    let mut args = Vec::new();
    let mut start = open_paren + 1;
    let mut index = start;
    let mut paren_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut bracket_depth = 0usize;

    while index < close_paren {
        if let Some(next_index) = skip_comment(content, index) {
            index = next_index;
            continue;
        }

        match bytes[index] {
            b'\'' | b'"' | b'`' => {
                if let Some(next_index) = skip_quoted_string(content, index) {
                    index = next_index;
                } else {
                    break;
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
                brace_depth = brace_depth.saturating_sub(1);
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
                push_trimmed_arg(content, start, index, &mut args);
                start = index + 1;
                index += 1;
            }
            _ => index += 1,
        }
    }

    push_trimmed_arg(content, start, close_paren, &mut args);
    args
}

fn push_trimmed_arg(content: &str, start: usize, end: usize, args: &mut Vec<ArgSpan>) {
    let bytes = content.as_bytes();
    let mut start = start;
    let mut end = end;

    while bytes
        .get(start)
        .is_some_and(|byte| byte.is_ascii_whitespace())
    {
        start += 1;
    }
    while end > start
        && bytes
            .get(end - 1)
            .is_some_and(|byte| byte.is_ascii_whitespace())
    {
        end -= 1;
    }

    if start < end {
        args.push(ArgSpan { start, end });
    }
}

fn read_string_literal_in_span(content: &str, arg: &ArgSpan) -> Option<Literal> {
    let literal = read_string_literal_at(content, arg.start)?;
    if literal.end <= arg.end {
        Some(literal)
    } else {
        None
    }
}

fn read_dynamic_import_arg(content: &str, arg: &ArgSpan) -> Option<Literal> {
    let bytes = content.as_bytes();
    let mut index = arg.start.min(content.len());
    let end = arg.end.min(content.len());

    while index < end {
        if let Some(next_index) = skip_comment(content, index) {
            index = next_index;
            continue;
        }

        if matches!(bytes[index], b'\'' | b'"' | b'`') {
            index = skip_quoted_string(content, index)?;
            continue;
        }

        if matches_identifier_at(content, index, "import") {
            let open_paren = skip_whitespace_and_comments(content, index + "import".len());
            if open_paren < end && bytes.get(open_paren) == Some(&b'(') {
                let literal = read_string_literals_in_call(content, open_paren, 1)
                    .into_iter()
                    .next()?;
                if literal.end <= end {
                    return Some(literal);
                }
            }
        }

        index += 1;
    }

    None
}

fn object_arg_open(content: &str, arg: &ArgSpan) -> Option<usize> {
    let index = skip_whitespace_and_comments(content, arg.start);
    if index < arg.end && content.as_bytes().get(index) == Some(&b'{') {
        Some(index)
    } else {
        None
    }
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

fn read_identifier_at(content: &str, start: usize) -> Option<Literal> {
    let bytes = content.as_bytes();
    let start = skip_whitespace_and_comments(content, start.min(content.len()));
    let first = *bytes.get(start)?;
    if !(first.is_ascii_alphabetic() || matches!(first, b'_' | b'$')) {
        return None;
    }

    let mut end = start + 1;
    while bytes
        .get(end)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'_' | b'$' | b'-'))
    {
        end += 1;
    }

    Some(Literal {
        value: content[start..end].to_owned(),
        start,
        end,
    })
}

fn matches_identifier_at(content: &str, start: usize, expected: &str) -> bool {
    let Some(identifier) = read_identifier_at(content, start) else {
        return false;
    };

    identifier.start == start && identifier.value == expected
}

fn previous_non_whitespace_byte(content: &str, start: usize, min: usize) -> Option<u8> {
    let bytes = content.as_bytes();
    let mut index = start;

    while index > min {
        index -= 1;
        if !bytes[index].is_ascii_whitespace() {
            return Some(bytes[index]);
        }
    }

    None
}

fn simple_identifier_expression(content: &str, start: usize, end: usize) -> Option<Literal> {
    let identifier = read_identifier_at(content, start)?;
    let tail = skip_whitespace_and_comments(content, identifier.end);
    if identifier.start >= start && tail >= end {
        Some(identifier)
    } else {
        None
    }
}

fn split_v_for_expression(value: &str) -> Option<ForExpression> {
    let separator = value
        .find(" in ")
        .map(|index| (index, 4))
        .or_else(|| value.find(" of ").map(|index| (index, 4)))?;
    let aliases = value[..separator.0]
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')')
        .split(',')
        .map(str::trim)
        .filter(|alias| looks_like_component_member_name(alias))
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let iterable_start = separator.0 + separator.1;
    let iterable_end = value.len();

    Some(ForExpression {
        aliases,
        iterable_start,
        iterable_end,
    })
}

fn collect_top_level_object_properties(content: &str, open_brace: usize) -> Vec<ObjectProperty> {
    let Some(close_brace) = find_matching_delimiter(content, open_brace, b'{', b'}') else {
        return Vec::new();
    };

    let bytes = content.as_bytes();
    let mut properties = Vec::new();
    let mut index = open_brace + 1;

    while index < close_brace {
        index = skip_whitespace_and_comments(content, index);

        while bytes.get(index).is_some_and(|byte| *byte == b',') {
            index += 1;
            index = skip_whitespace_and_comments(content, index);
        }

        let Some(mut key) = read_object_property_key(content, index) else {
            index += 1;
            continue;
        };
        index = key.end;

        index = skip_whitespace_and_comments(content, index);
        if is_object_method_modifier(&key.value)
            && let Some(method_key) = read_object_property_key(content, index)
        {
            let method_tail = skip_whitespace_and_comments(content, method_key.end);
            if bytes.get(method_tail) == Some(&b'(') {
                key = method_key;
                index = method_tail;
            }
        }

        if bytes.get(index) == Some(&b':') {
            index += 1;
            let value_start = skip_whitespace_and_comments(content, index);
            let value_end = skip_top_level_value(content, value_start, close_brace);
            properties.push(ObjectProperty {
                key,
                value_start,
                value_end,
                shorthand: false,
            });
            index = value_end;
        } else if bytes.get(index) == Some(&b'(') {
            let Some(params_end) = find_matching_delimiter(content, index, b'(', b')') else {
                index += 1;
                continue;
            };
            let body_start = skip_whitespace_and_comments(content, params_end + 1);
            if bytes.get(body_start) != Some(&b'{') {
                index = params_end + 1;
                continue;
            }
            let Some(body_end) = find_matching_delimiter(content, body_start, b'{', b'}') else {
                index = body_start + 1;
                continue;
            };
            properties.push(ObjectProperty {
                value_start: key.start,
                value_end: body_end + 1,
                key,
                shorthand: false,
            });
            index = body_end + 1;
        } else {
            let value_end = skip_top_level_value(content, index, close_brace);
            properties.push(ObjectProperty {
                value_start: key.start,
                value_end,
                key,
                shorthand: true,
            });
            index = value_end;
        }
    }

    properties
}

fn collect_top_level_object_keys(content: &str, open_brace: usize) -> Vec<Literal> {
    collect_top_level_object_properties(content, open_brace)
        .into_iter()
        .map(|property| property.key)
        .collect()
}

fn read_object_property_key(content: &str, index: usize) -> Option<Literal> {
    let bytes = content.as_bytes();
    if bytes
        .get(index)
        .is_some_and(|byte| matches!(*byte, b'\'' | b'"' | b'`'))
    {
        return read_string_literal_at(content, index);
    }

    read_identifier_at(content, index)
}

fn is_object_method_modifier(value: &str) -> bool {
    matches!(value, "async" | "get" | "set")
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

fn looks_like_component_member_name(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('$')
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '$' | '-'))
}

fn looks_like_twig_template_import(value: &str) -> bool {
    value.ends_with(".twig")
}

fn is_reserved_template_identifier(value: &str) -> bool {
    value.starts_with('$')
        || matches!(
            value,
            "true"
                | "false"
                | "null"
                | "undefined"
                | "this"
                | "Math"
                | "Date"
                | "JSON"
                | "Number"
                | "String"
                | "Array"
                | "Object"
                | "Boolean"
                | "parseInt"
                | "parseFloat"
                | "isNaN"
                | "NaN"
                | "Infinity"
                | "return"
                | "typeof"
                | "new"
                | "in"
                | "of"
                | "as"
        )
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

    #[test]
    fn extracts_component_option_member_definitions() {
        let content = r#"
Shopware.Component.register('sw-product-detail', {
    data() {
        return {
            isLoading: false,
        };
    },
    computed: {
        product() {
            return {};
        },
    },
    methods: {
        onSave() {},
        async createdComponent() {},
    },
});
"#;

        let facts = extract(
            content,
            Path::new("administration/src/module/sw-product/index.js"),
            FactRole::Definition,
            true,
        );
        let surfaces = surfaces(&facts);

        assert!(
            surfaces.contains(&"admin:component:property:sw-product-detail:isLoading".to_string())
        );
        assert!(
            surfaces.contains(&"admin:component:property:sw-product-detail:product".to_string())
        );
        assert!(surfaces.contains(&"admin:component:method:sw-product-detail:onSave".to_string()));
        assert!(
            surfaces
                .contains(&"admin:component:method:sw-product-detail:createdComponent".to_string())
        );
        assert!(!surfaces.contains(&"admin:component:method:sw-product-detail:async".to_string()));
        assert!(facts.iter().all(|fact| fact.role == FactRole::Definition));
    }

    #[test]
    fn extracts_component_extension_member_usages_from_options_and_inline_template() {
        let content = r#"
Shopware.Component.override('sw-product-detail', {
    template: `
        <template>
            <div v-if="isLoading">
                {{ product.name }}
                <button @click="onSave" :disabled="!canSave">Save</button>
                <span v-for="item in items">{{ item.name }}</span>
            </div>
        </template>
    `,
    data() {
        return {
            isLoading: false,
        };
    },
    computed: {
        product() {
            return this.product;
        },
    },
    methods: {
        async loadAll() {},
        onSave() {
            this.$super('onSave');
            this.saveProduct();
        },
    },
});
"#;

        let facts = extract(
            content,
            Path::new("administration/src/main.js"),
            FactRole::Usage,
            false,
        );
        let surfaces = surfaces(&facts);

        assert!(
            surfaces.contains(&"admin:component:property:sw-product-detail:isLoading".to_string())
        );
        assert!(
            surfaces.contains(&"admin:component:property:sw-product-detail:product".to_string())
        );
        assert!(
            surfaces.contains(&"admin:component:property:sw-product-detail:canSave".to_string())
        );
        assert!(surfaces.contains(&"admin:component:property:sw-product-detail:items".to_string()));
        assert!(surfaces.contains(&"admin:component:method:sw-product-detail:loadAll".to_string()));
        assert!(surfaces.contains(&"admin:component:method:sw-product-detail:onSave".to_string()));
        assert!(
            surfaces.contains(&"admin:component:method:sw-product-detail:saveProduct".to_string())
        );
        assert!(!surfaces.contains(&"admin:component:method:sw-product-detail:async".to_string()));
    }

    #[test]
    fn extracts_component_extension_member_usages_from_imported_template() {
        let content = r#"
import template from './sw-product-detail.html.twig';

Shopware.Component.extend('swag-product-detail', 'sw-product-detail', {
    template,
});
"#;
        let mut related_files = |source: &Path, import: &str| {
            assert_eq!(source, Path::new("administration/src/main.js"));
            assert_eq!(import, "./sw-product-detail.html.twig");
            Ok(Some((
                Path::new("administration/src/sw-product-detail.html.twig").to_path_buf(),
                r#"
<template>
    <div v-if="product">
        {{ product.name }}
        <button @click="onSave()">Save</button>
    </div>
</template>
"#
                .to_owned(),
            )))
        };

        let facts = extract_with_related(
            content,
            Path::new("administration/src/main.js"),
            FactRole::Usage,
            true,
            &mut related_files,
        )
        .expect("extract with related template");
        let surfaces = surfaces(&facts);

        assert!(
            surfaces.contains(&"admin:component:property:sw-product-detail:product".to_string())
        );
        assert!(surfaces.contains(&"admin:component:method:sw-product-detail:onSave".to_string()));
        assert!(facts.iter().any(|fact| {
            fact.surface.as_str() == "admin:component:property:sw-product-detail:product"
                && fact.evidence.path == Path::new("administration/src/sw-product-detail.html.twig")
        }));
    }
}
