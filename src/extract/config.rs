use std::path::Path;

use anyhow::Result;
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use serde_json::Value as JsonValue;
use serde_yaml::Value as YamlValue;
use toml::Value as TomlValue;

use crate::model::{Confidence, Evidence, Fact, FactRole, line_col_at, line_snippet};

pub fn extract_xml(
    content: &str,
    relative_path: &Path,
    role: FactRole,
    include_snippets: bool,
) -> Result<Vec<Fact>> {
    let mut reader = Reader::from_str(content);
    reader.config_mut().trim_text(true);

    let mut facts = FactCollector::new();
    let mut stack = Vec::new();

    while let Ok(event) = reader.read_event() {
        let event_end = reader.buffer_position() as usize;

        match event {
            Event::Start(start) => {
                let name = local_xml_name(start.name().as_ref());
                handle_xml_element(
                    content,
                    relative_path,
                    role,
                    include_snippets,
                    &name,
                    &start,
                    event_end,
                    &mut facts,
                );
                stack.push(name);
            }
            Event::Empty(start) => {
                let name = local_xml_name(start.name().as_ref());
                handle_xml_element(
                    content,
                    relative_path,
                    role,
                    include_snippets,
                    &name,
                    &start,
                    event_end,
                    &mut facts,
                );
            }
            Event::Text(text) => {
                if let Ok(value) = text.decode() {
                    handle_xml_text(
                        value.trim(),
                        content,
                        relative_path,
                        role,
                        include_snippets,
                        &stack,
                        event_end,
                        &mut facts,
                    );
                }
            }
            Event::CData(text) => {
                if let Ok(value) = text.decode() {
                    handle_xml_text(
                        value.trim(),
                        content,
                        relative_path,
                        role,
                        include_snippets,
                        &stack,
                        event_end,
                        &mut facts,
                    );
                }
            }
            Event::End(_) => {
                stack.pop();
            }
            Event::Eof => break,
            _ => {}
        }
    }

    Ok(facts.into_vec())
}

pub fn extract_json(
    content: &str,
    relative_path: &Path,
    role: FactRole,
    include_snippets: bool,
) -> Result<Vec<Fact>> {
    let Ok(value) = serde_json::from_str::<JsonValue>(content) else {
        return Ok(Vec::new());
    };

    let mut facts = FactCollector::new();
    walk_json(
        &value,
        &mut Vec::new(),
        content,
        relative_path,
        role,
        include_snippets,
        &mut facts,
    );
    Ok(facts.into_vec())
}

pub fn extract_yaml(
    content: &str,
    relative_path: &Path,
    role: FactRole,
    include_snippets: bool,
) -> Result<Vec<Fact>> {
    let Ok(value) = serde_yaml::from_str::<YamlValue>(content) else {
        return Ok(Vec::new());
    };

    let mut facts = FactCollector::new();
    walk_yaml(
        &value,
        &mut Vec::new(),
        content,
        relative_path,
        role,
        include_snippets,
        &mut facts,
    );
    Ok(facts.into_vec())
}

pub fn extract_toml(
    content: &str,
    relative_path: &Path,
    role: FactRole,
    include_snippets: bool,
) -> Result<Vec<Fact>> {
    let Ok(value) = toml::from_str::<TomlValue>(content) else {
        return Ok(Vec::new());
    };

    let mut facts = FactCollector::new();
    walk_toml(
        &value,
        &mut Vec::new(),
        content,
        relative_path,
        role,
        include_snippets,
        &mut facts,
    );
    Ok(facts.into_vec())
}

#[allow(clippy::too_many_arguments)]
fn handle_xml_element(
    content: &str,
    relative_path: &Path,
    role: FactRole,
    include_snippets: bool,
    element: &str,
    start: &BytesStart<'_>,
    event_end: usize,
    facts: &mut FactCollector,
) {
    let attrs = xml_attrs(start);
    let event_start = xml_event_start(content, event_end);

    if element == "service" {
        emit_attr(
            &attrs,
            "id",
            "service:id:",
            "xml.service.id",
            Confidence::High,
            content,
            relative_path,
            role,
            include_snippets,
            event_start,
            event_end,
            facts,
        );
        emit_attr(
            &attrs,
            "decorates",
            "service:id:",
            "xml.service.decorates",
            Confidence::High,
            content,
            relative_path,
            role,
            include_snippets,
            event_start,
            event_end,
            facts,
        );
        emit_attr(
            &attrs,
            "parent",
            "service:id:",
            "xml.service.parent",
            Confidence::Medium,
            content,
            relative_path,
            role,
            include_snippets,
            event_start,
            event_end,
            facts,
        );
        emit_attr(
            &attrs,
            "class",
            "php:class:",
            "xml.service.class",
            Confidence::Medium,
            content,
            relative_path,
            role,
            include_snippets,
            event_start,
            event_end,
            facts,
        );
    }

    if element == "argument"
        && attr_value(&attrs, "type")
            .is_some_and(|value| value == "service" || value == "tagged_iterator")
    {
        emit_attr(
            &attrs,
            "id",
            "service:id:",
            "xml.service.reference",
            Confidence::High,
            content,
            relative_path,
            role,
            include_snippets,
            event_start,
            event_end,
            facts,
        );
    }

    if element == "tag" {
        emit_attr(
            &attrs,
            "name",
            "service:id:",
            "xml.service.tag",
            Confidence::Medium,
            content,
            relative_path,
            role,
            include_snippets,
            event_start,
            event_end,
            facts,
        );
    }

    if element == "route" {
        emit_attr(
            &attrs,
            "id",
            "route:name:",
            "xml.route.id",
            Confidence::High,
            content,
            relative_path,
            role,
            include_snippets,
            event_start,
            event_end,
            facts,
        );
        emit_attr(
            &attrs,
            "name",
            "route:name:",
            "xml.route.name",
            Confidence::High,
            content,
            relative_path,
            role,
            include_snippets,
            event_start,
            event_end,
            facts,
        );

        if let Some(path) = attr_value(&attrs, "path").filter(|value| looks_like_api_path(value)) {
            emit_value(
                format!("api:{path}"),
                "xml.route.api-path",
                Confidence::Medium,
                path,
                content,
                relative_path,
                role,
                include_snippets,
                event_start,
                event_end,
                facts,
            );
        }
    }

    for key in ["event", "events"] {
        if let Some(value) = attr_value(&attrs, key) {
            for event_name in split_config_values(value) {
                emit_event_value(
                    event_name,
                    "xml.event",
                    content,
                    relative_path,
                    role,
                    include_snippets,
                    event_start,
                    event_end,
                    facts,
                );
            }
        }
    }

    for key in ["listener", "subscriber", "service"] {
        emit_attr(
            &attrs,
            key,
            "service:id:",
            "xml.service.listener",
            Confidence::Medium,
            content,
            relative_path,
            role,
            include_snippets,
            event_start,
            event_end,
            facts,
        );
    }

    for key in ["entity", "entity-name", "entityName"] {
        emit_attr(
            &attrs,
            key,
            "dal:entity:",
            "xml.entity",
            Confidence::High,
            content,
            relative_path,
            role,
            include_snippets,
            event_start,
            event_end,
            facts,
        );
    }

    if matches!(element, "theme" | "app") {
        let prefix = if element == "theme" {
            "theme:name:"
        } else {
            "app:name:"
        };
        emit_attr(
            &attrs,
            "name",
            prefix,
            "xml.metadata.name",
            Confidence::Medium,
            content,
            relative_path,
            role,
            include_snippets,
            event_start,
            event_end,
            facts,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_xml_text(
    value: &str,
    content: &str,
    relative_path: &Path,
    role: FactRole,
    include_snippets: bool,
    stack: &[String],
    event_end: usize,
    facts: &mut FactCollector,
) {
    if value.is_empty() {
        return;
    }

    let element = stack.last().map(String::as_str).unwrap_or_default();
    let event_start = find_value_before(content, value, event_end).unwrap_or_else(|| {
        event_end
            .saturating_sub(value.len())
            .min(content.len().saturating_sub(1))
    });

    if element == "name"
        && stack
            .iter()
            .any(|part| matches!(part.as_str(), "meta" | "metadata"))
    {
        emit_value(
            format!("app:name:{value}"),
            "xml.metadata.name",
            Confidence::Medium,
            value,
            content,
            relative_path,
            role,
            include_snippets,
            event_start,
            event_end,
            facts,
        );
    }

    if matches!(element, "read" | "create" | "update" | "delete" | "entity")
        && looks_like_entity_name(value)
    {
        emit_value(
            format!("dal:entity:{value}"),
            "xml.entity",
            Confidence::High,
            value,
            content,
            relative_path,
            role,
            include_snippets,
            event_start,
            event_end,
            facts,
        );
    }

    if element == "event" {
        emit_event_value(
            value,
            "xml.event",
            content,
            relative_path,
            role,
            include_snippets,
            event_start,
            event_end,
            facts,
        );
    }

    if element == "route" && looks_like_route_name(value) {
        emit_value(
            format!("route:name:{value}"),
            "xml.route.text",
            Confidence::Medium,
            value,
            content,
            relative_path,
            role,
            include_snippets,
            event_start,
            event_end,
            facts,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn walk_json(
    value: &JsonValue,
    path: &mut Vec<String>,
    content: &str,
    relative_path: &Path,
    role: FactRole,
    include_snippets: bool,
    facts: &mut FactCollector,
) {
    match value {
        JsonValue::Object(map) => {
            for (key, child) in map {
                handle_mapping_key(
                    key,
                    ValueShape::Json(child),
                    path,
                    content,
                    relative_path,
                    role,
                    include_snippets,
                    facts,
                );
                path.push(key.clone());
                walk_json(
                    child,
                    path,
                    content,
                    relative_path,
                    role,
                    include_snippets,
                    facts,
                );
                path.pop();
            }
        }
        JsonValue::Array(values) => {
            for child in values {
                walk_json(
                    child,
                    path,
                    content,
                    relative_path,
                    role,
                    include_snippets,
                    facts,
                );
            }
        }
        JsonValue::String(value) => handle_config_string(
            value,
            path,
            content,
            relative_path,
            role,
            include_snippets,
            facts,
        ),
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn walk_yaml(
    value: &YamlValue,
    path: &mut Vec<String>,
    content: &str,
    relative_path: &Path,
    role: FactRole,
    include_snippets: bool,
    facts: &mut FactCollector,
) {
    match value {
        YamlValue::Mapping(map) => {
            for (key, child) in map {
                let Some(key) = yaml_key(key) else {
                    continue;
                };
                handle_mapping_key(
                    &key,
                    ValueShape::Yaml(child),
                    path,
                    content,
                    relative_path,
                    role,
                    include_snippets,
                    facts,
                );
                path.push(key);
                walk_yaml(
                    child,
                    path,
                    content,
                    relative_path,
                    role,
                    include_snippets,
                    facts,
                );
                path.pop();
            }
        }
        YamlValue::Sequence(values) => {
            for child in values {
                walk_yaml(
                    child,
                    path,
                    content,
                    relative_path,
                    role,
                    include_snippets,
                    facts,
                );
            }
        }
        YamlValue::String(value) => handle_config_string(
            value,
            path,
            content,
            relative_path,
            role,
            include_snippets,
            facts,
        ),
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn walk_toml(
    value: &TomlValue,
    path: &mut Vec<String>,
    content: &str,
    relative_path: &Path,
    role: FactRole,
    include_snippets: bool,
    facts: &mut FactCollector,
) {
    match value {
        TomlValue::Table(map) => {
            for (key, child) in map {
                handle_mapping_key(
                    key,
                    ValueShape::Toml(child),
                    path,
                    content,
                    relative_path,
                    role,
                    include_snippets,
                    facts,
                );
                path.push(key.clone());
                walk_toml(
                    child,
                    path,
                    content,
                    relative_path,
                    role,
                    include_snippets,
                    facts,
                );
                path.pop();
            }
        }
        TomlValue::Array(values) => {
            for child in values {
                walk_toml(
                    child,
                    path,
                    content,
                    relative_path,
                    role,
                    include_snippets,
                    facts,
                );
            }
        }
        TomlValue::String(value) => handle_config_string(
            value,
            path,
            content,
            relative_path,
            role,
            include_snippets,
            facts,
        ),
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_mapping_key(
    key: &str,
    value: ValueShape<'_>,
    path: &[String],
    content: &str,
    relative_path: &Path,
    role: FactRole,
    include_snippets: bool,
    facts: &mut FactCollector,
) {
    if is_shopware_package(key) && path.iter().any(|part| is_composer_dependency_section(part)) {
        emit_value_at_search_offset(
            format!("composer:constraint:{key}"),
            "composer.constraint",
            Confidence::High,
            key,
            content,
            relative_path,
            role,
            include_snippets,
            facts,
        );
    }

    if is_service_definition_key(key, path, value) {
        emit_value_at_search_offset(
            format!("service:id:{key}"),
            "config.service.id",
            Confidence::High,
            key,
            content,
            relative_path,
            role,
            include_snippets,
            facts,
        );
    }

    if is_route_definition_key(key, path, value) {
        emit_value_at_search_offset(
            format!("route:name:{key}"),
            "config.route.name",
            Confidence::High,
            key,
            content,
            relative_path,
            role,
            include_snippets,
            facts,
        );
    }

    if path
        .last()
        .is_some_and(|part| normalize_key(part) == "entities")
        && looks_like_entity_name(key)
    {
        emit_value_at_search_offset(
            format!("dal:entity:{key}"),
            "config.entity.key",
            Confidence::Medium,
            key,
            content,
            relative_path,
            role,
            include_snippets,
            facts,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_config_string(
    value: &str,
    path: &[String],
    content: &str,
    relative_path: &Path,
    role: FactRole,
    include_snippets: bool,
    facts: &mut FactCollector,
) {
    if value.trim().is_empty() {
        return;
    }

    let key = path
        .last()
        .map(|part| normalize_key(part))
        .unwrap_or_default();
    let offset = find_value_offset(content, value).unwrap_or(0);
    let event_end = offset.saturating_add(value.len());

    if is_theme_file(relative_path) && key == "name" {
        emit_value(
            format!("theme:name:{value}"),
            "theme.metadata.name",
            Confidence::Medium,
            value,
            content,
            relative_path,
            role,
            include_snippets,
            offset,
            event_end,
            facts,
        );
    }

    if is_app_file(relative_path) && key == "name" {
        emit_value(
            format!("app:name:{value}"),
            "app.metadata.name",
            Confidence::Medium,
            value,
            content,
            relative_path,
            role,
            include_snippets,
            offset,
            event_end,
            facts,
        );
    }

    if looks_like_twig_template(value)
        || (path.iter().any(|part| normalize_key(part) == "views") && value.starts_with('@'))
    {
        emit_value(
            format!("twig:template:{value}"),
            "theme.view",
            Confidence::Medium,
            value,
            content,
            relative_path,
            role,
            include_snippets,
            offset,
            event_end,
            facts,
        );
    }

    if (is_route_context(&key, path) || looks_like_route_name(value))
        && looks_like_route_name(value)
    {
        emit_value(
            format!("route:name:{value}"),
            "config.route.string",
            Confidence::Medium,
            value,
            content,
            relative_path,
            role,
            include_snippets,
            offset,
            event_end,
            facts,
        );
    }

    if looks_like_api_path(value) {
        emit_value(
            format!("api:{value}"),
            "config.api-path",
            Confidence::Medium,
            value,
            content,
            relative_path,
            role,
            include_snippets,
            offset,
            event_end,
            facts,
        );
    }

    if is_service_context(&key, path) && looks_like_service_id(value) {
        emit_value(
            format!("service:id:{value}"),
            "config.service.string",
            Confidence::Medium,
            value,
            content,
            relative_path,
            role,
            include_snippets,
            offset,
            event_end,
            facts,
        );
    }

    if key == "name" && path.iter().any(|part| normalize_key(part) == "tags") {
        emit_value(
            format!("service:id:{value}"),
            "config.service.tag",
            Confidence::Medium,
            value,
            content,
            relative_path,
            role,
            include_snippets,
            offset,
            event_end,
            facts,
        );
    }

    if is_event_context(&key, path) {
        for event_name in split_config_values(value) {
            emit_event_value(
                event_name,
                "config.event",
                content,
                relative_path,
                role,
                include_snippets,
                offset,
                event_end,
                facts,
            );
        }
    }

    if is_entity_context(&key, path) && looks_like_entity_name(value) {
        emit_value(
            format!("dal:entity:{value}"),
            "config.entity",
            Confidence::High,
            value,
            content,
            relative_path,
            role,
            include_snippets,
            offset,
            event_end,
            facts,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_attr(
    attrs: &[(String, String)],
    key: &str,
    prefix: &str,
    usage_kind: &str,
    confidence: Confidence,
    content: &str,
    relative_path: &Path,
    role: FactRole,
    include_snippets: bool,
    event_start: usize,
    event_end: usize,
    facts: &mut FactCollector,
) {
    if let Some(value) = attrs
        .iter()
        .find_map(|(attr_key, value)| (attr_key == key).then_some(value))
        .filter(|value| !value.trim().is_empty())
    {
        emit_value(
            format!("{prefix}{value}"),
            usage_kind,
            confidence,
            value,
            content,
            relative_path,
            role,
            include_snippets,
            event_start,
            event_end,
            facts,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_event_value(
    value: &str,
    usage_kind: &str,
    content: &str,
    relative_path: &Path,
    role: FactRole,
    include_snippets: bool,
    event_start: usize,
    event_end: usize,
    facts: &mut FactCollector,
) {
    if value.trim().is_empty() {
        return;
    }

    let prefix = if looks_like_php_class(value) {
        "event:class:"
    } else {
        "event:name:"
    };

    emit_value(
        format!("{prefix}{value}"),
        usage_kind,
        Confidence::High,
        value,
        content,
        relative_path,
        role,
        include_snippets,
        event_start,
        event_end,
        facts,
    );
}

#[allow(clippy::too_many_arguments)]
fn emit_value_at_search_offset(
    surface: String,
    usage_kind: &str,
    confidence: Confidence,
    value: &str,
    content: &str,
    relative_path: &Path,
    role: FactRole,
    include_snippets: bool,
    facts: &mut FactCollector,
) {
    let offset = find_value_offset(content, value).unwrap_or(0);
    emit_value(
        surface,
        usage_kind,
        confidence,
        value,
        content,
        relative_path,
        role,
        include_snippets,
        offset,
        offset.saturating_add(value.len()),
        facts,
    );
}

#[allow(clippy::too_many_arguments)]
fn emit_value(
    surface: String,
    usage_kind: &str,
    confidence: Confidence,
    value: &str,
    content: &str,
    relative_path: &Path,
    role: FactRole,
    include_snippets: bool,
    event_start: usize,
    event_end: usize,
    facts: &mut FactCollector,
) {
    let offset = find_between(content, value, event_start, event_end)
        .or_else(|| find_value_offset(content, value))
        .unwrap_or(event_start);

    facts.push(
        role,
        surface,
        evidence(content, relative_path, offset, include_snippets),
        confidence,
        usage_kind,
    );
}

fn xml_attrs(start: &BytesStart<'_>) -> Vec<(String, String)> {
    start
        .attributes()
        .filter_map(|attribute| {
            let attribute = attribute.ok()?;
            let key = local_xml_name(attribute.key.as_ref());
            let value = attribute.unescape_value().ok()?.into_owned();
            Some((key, value))
        })
        .collect()
}

fn attr_value<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find_map(|(attr_key, value)| (attr_key == key).then_some(value.as_str()))
}

fn local_xml_name(bytes: &[u8]) -> String {
    std::str::from_utf8(bytes)
        .unwrap_or_default()
        .rsplit(':')
        .next()
        .unwrap_or_default()
        .to_string()
}

fn xml_event_start(content: &str, event_end: usize) -> usize {
    let bounded_end = event_end.min(content.len());
    content[..bounded_end].rfind('<').unwrap_or(bounded_end)
}

fn yaml_key(value: &YamlValue) -> Option<String> {
    match value {
        YamlValue::String(value) => Some(value.clone()),
        YamlValue::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum ValueShape<'a> {
    Json(&'a JsonValue),
    Yaml(&'a YamlValue),
    Toml(&'a TomlValue),
}

impl ValueShape<'_> {
    fn is_mapping(self) -> bool {
        match self {
            Self::Json(value) => value.is_object(),
            Self::Yaml(YamlValue::Mapping(_)) | Self::Toml(TomlValue::Table(_)) => true,
            _ => false,
        }
    }

    fn has_key(self, expected: &str) -> bool {
        match self {
            Self::Json(JsonValue::Object(map)) => {
                map.keys().any(|key| normalize_key(key) == expected)
            }
            Self::Yaml(YamlValue::Mapping(map)) => map
                .keys()
                .filter_map(yaml_key)
                .any(|key| normalize_key(&key) == expected),
            Self::Toml(TomlValue::Table(map)) => {
                map.keys().any(|key| normalize_key(key) == expected)
            }
            _ => false,
        }
    }
}

fn is_service_definition_key(key: &str, path: &[String], value: ValueShape<'_>) -> bool {
    let normalized = normalize_key(key);

    if key.starts_with('_') || matches!(normalized.as_str(), "defaults" | "instanceof") {
        return false;
    }

    path.last()
        .is_some_and(|part| normalize_key(part) == "services")
        && value.is_mapping()
        && (looks_like_service_id(key) || looks_like_php_class(key))
}

fn is_route_definition_key(key: &str, path: &[String], value: ValueShape<'_>) -> bool {
    if key.starts_with('_') || !looks_like_route_name(key) {
        return false;
    }

    path.last()
        .is_some_and(|part| normalize_key(part) == "routes")
        || (path.is_empty() && value.is_mapping() && value.has_key("path"))
}

fn is_composer_dependency_section(value: &str) -> bool {
    matches!(
        normalize_key(value).as_str(),
        "require" | "requiredev" | "conflict" | "replace" | "provide"
    )
}

fn is_shopware_package(value: &str) -> bool {
    matches!(
        value,
        "shopware/core" | "shopware/storefront" | "shopware/administration" | "shopware/platform"
    ) || value.starts_with("shopware/")
}

fn is_route_context(key: &str, path: &[String]) -> bool {
    key.contains("route")
        || key == "parentpath"
        || key == "routename"
        || path.iter().any(|part| normalize_key(part) == "routes")
}

fn is_service_context(key: &str, path: &[String]) -> bool {
    matches!(
        key,
        "service" | "serviceid" | "decorates" | "parent" | "factory" | "controller"
    ) || path.iter().any(|part| normalize_key(part) == "services")
}

fn is_event_context(key: &str, path: &[String]) -> bool {
    key.contains("event")
        || path
            .iter()
            .any(|part| normalize_key(part).contains("events"))
}

fn is_entity_context(key: &str, path: &[String]) -> bool {
    key.contains("entity")
        || key == "entities"
        || path.iter().any(|part| normalize_key(part) == "entities")
}

fn normalize_key(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn looks_like_route_name(value: &str) -> bool {
    let value = value.trim();
    value.contains('.')
        && value.chars().any(|ch| ch.is_ascii_alphabetic())
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
        && !value.starts_with('.')
        && !value.ends_with('.')
}

fn looks_like_service_id(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && !value.starts_with('@')
        && !value.starts_with('%')
        && !value.contains(' ')
        && (value.contains('.')
            || value.contains('\\')
            || value.contains('_')
            || value.contains('-'))
}

fn looks_like_php_class(value: &str) -> bool {
    value.contains('\\')
        && value.split('\\').all(|part| {
            part.chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_uppercase())
        })
}

fn looks_like_entity_name(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '_' | '-'))
        && value.chars().any(|ch| ch.is_ascii_lowercase())
}

fn looks_like_twig_template(value: &str) -> bool {
    value.starts_with('@') || value.ends_with(".twig") || value.contains(".html.twig")
}

fn looks_like_api_path(value: &str) -> bool {
    value.starts_with("/api/") || value.starts_with("/store-api/")
}

fn is_theme_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name == "theme.json"
                || name == "theme.yaml"
                || name == "theme.yml"
                || name == "theme.toml"
        })
}

fn is_app_file(path: &Path) -> bool {
    path.components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .is_some_and(|part| part.eq_ignore_ascii_case("app"))
    })
}

fn split_config_values(value: &str) -> impl Iterator<Item = &str> {
    value
        .split([',', ' '])
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn find_value_offset(content: &str, value: &str) -> Option<usize> {
    if value.is_empty() {
        return None;
    }

    let quoted_double = format!("\"{value}\"");
    if let Some(offset) = content.find(&quoted_double) {
        return Some(offset + 1);
    }

    let quoted_single = format!("'{value}'");
    if let Some(offset) = content.find(&quoted_single) {
        return Some(offset + 1);
    }

    content.find(value)
}

fn find_between(content: &str, value: &str, start: usize, end: usize) -> Option<usize> {
    if value.is_empty() || start >= content.len() {
        return None;
    }

    let bounded_end = end.min(content.len());
    if start >= bounded_end {
        return None;
    }

    content[start..bounded_end]
        .find(value)
        .map(|offset| start + offset)
}

fn find_value_before(content: &str, value: &str, end: usize) -> Option<usize> {
    if value.is_empty() {
        return None;
    }

    let bounded_end = end.min(content.len());
    content[..bounded_end].rfind(value)
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
    fn extracts_xml_services_routes_events_and_entities() {
        let content = r#"
<container>
  <services>
    <service id="Swag\Service\Foo" decorates="shopware.cart.processor">
      <argument type="service" id="product.repository"/>
      <tag name="kernel.event_listener" event="checkout.order.placed"/>
    </service>
  </services>
  <routes>
    <route id="frontend.foo.page" path="/api/foo"/>
  </routes>
  <permissions><read>product</read></permissions>
</container>
"#;

        let facts = extract_xml(
            content,
            Path::new("Resources/config/services.xml"),
            FactRole::Usage,
            true,
        )
        .unwrap();
        let surfaces = surfaces(&facts);

        assert!(surfaces.contains(&"service:id:Swag\\Service\\Foo".to_string()));
        assert!(surfaces.contains(&"service:id:shopware.cart.processor".to_string()));
        assert!(surfaces.contains(&"service:id:product.repository".to_string()));
        assert!(surfaces.contains(&"service:id:kernel.event_listener".to_string()));
        assert!(surfaces.contains(&"event:name:checkout.order.placed".to_string()));
        assert!(surfaces.contains(&"route:name:frontend.foo.page".to_string()));
        assert!(surfaces.contains(&"api:/api/foo".to_string()));
        assert!(surfaces.contains(&"dal:entity:product".to_string()));
        assert!(facts.iter().any(|fact| fact.evidence.snippet.is_some()));
    }

    #[test]
    fn malformed_xml_returns_partial_facts() {
        let facts = extract_xml(
            r#"<container><service id="valid.service"><broken></container>"#,
            Path::new("services.xml"),
            FactRole::Usage,
            false,
        )
        .unwrap();

        assert!(surfaces(&facts).contains(&"service:id:valid.service".to_string()));
    }

    #[test]
    fn extracts_json_composer_theme_and_structural_strings() {
        let content = r#"
{
  "require": { "shopware/core": "^6.6" },
  "name": "SwagTheme",
  "views": ["@Storefront"],
  "routes": { "frontend.foo.page": { "path": "/foo" } },
  "services": { "Swag\\Foo": { "decorates": "product.repository" } },
  "entity": "product"
}
"#;

        let facts = extract_json(
            content,
            Path::new("src/Resources/theme.json"),
            FactRole::Definition,
            false,
        )
        .unwrap();
        let surfaces = surfaces(&facts);

        assert!(surfaces.contains(&"composer:constraint:shopware/core".to_string()));
        assert!(surfaces.contains(&"theme:name:SwagTheme".to_string()));
        assert!(surfaces.contains(&"twig:template:@Storefront".to_string()));
        assert!(surfaces.contains(&"route:name:frontend.foo.page".to_string()));
        assert!(surfaces.contains(&"service:id:Swag\\Foo".to_string()));
        assert!(surfaces.contains(&"service:id:product.repository".to_string()));
        assert!(surfaces.contains(&"dal:entity:product".to_string()));
        assert!(facts.iter().all(|fact| fact.role == FactRole::Definition));
        assert_eq!(
            facts
                .iter()
                .find(|fact| fact.surface.as_str() == "route:name:frontend.foo.page")
                .map(|fact| fact.kind),
            Some(SurfaceKind::RouteName)
        );
    }

    #[test]
    fn extracts_yaml_and_toml_values() {
        let yaml = r#"
frontend.bar.page:
  path: /bar
services:
  foo.service:
    tags:
      - { name: kernel.event_subscriber, event: product.written }
    entity: category
"#;
        let toml = r#"
[theme]
name = "TomlTheme"
views = ["@Storefront", "@Plugins"]

[services."bar.service"]
decorates = "foo.service"
"#;

        let yaml_facts = extract_yaml(
            yaml,
            Path::new("Resources/config/routes.yaml"),
            FactRole::Usage,
            false,
        )
        .unwrap();
        let yaml_surfaces = surfaces(&yaml_facts);
        assert!(yaml_surfaces.contains(&"route:name:frontend.bar.page".to_string()));
        assert!(yaml_surfaces.contains(&"service:id:foo.service".to_string()));
        assert!(yaml_surfaces.contains(&"event:name:product.written".to_string()));
        assert!(yaml_surfaces.contains(&"dal:entity:category".to_string()));

        let toml_facts =
            extract_toml(toml, Path::new("theme.toml"), FactRole::Usage, false).unwrap();
        let toml_surfaces = surfaces(&toml_facts);
        assert!(toml_surfaces.contains(&"theme:name:TomlTheme".to_string()));
        assert!(toml_surfaces.contains(&"twig:template:@Storefront".to_string()));
        assert!(toml_surfaces.contains(&"service:id:bar.service".to_string()));
        assert!(toml_surfaces.contains(&"service:id:foo.service".to_string()));
    }
}
