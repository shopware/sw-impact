use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::OnceLock;

use anyhow::{Result, anyhow};
use regex::Regex;
use tree_sitter::{Node, Parser, Tree};

use crate::model::{Confidence, Evidence, Fact, FactRole, LineIndex, Parameter, Signature};

pub fn extract(
    content: &str,
    relative_path: &Path,
    role: FactRole,
    include_snippets: bool,
) -> Result<Vec<Fact>> {
    let context = PhpContext::build(content);
    let tree = if role == FactRole::Usage && should_parse_usage_ast(content, &context) {
        parse_php(content)?
    } else {
        None
    };
    let mut sink = FactSink::new(content, relative_path, include_snippets);

    match role {
        FactRole::Usage => extract_usages(content, tree.as_ref(), &context, &mut sink),
        FactRole::Definition => extract_definitions(content, &context, &mut sink),
    }

    Ok(sink.into_vec())
}

fn should_parse_usage_ast(content: &str, context: &PhpContext) -> bool {
    content.contains("Shopware\\")
        || context
            .import_facts
            .iter()
            .any(|import| is_shopware_fqcn(&import.fqcn))
}

fn parse_php(content: &str) -> Result<Option<Tree>> {
    thread_local! {
        static PARSER: RefCell<Option<Parser>> = const { RefCell::new(None) };
    }

    PARSER.with(|parser| {
        let mut parser = parser.borrow_mut();
        if parser.is_none() {
            let mut initialized = Parser::new();
            initialized
                .set_language(&tree_sitter_php::LANGUAGE_PHP.into())
                .map_err(|error| anyhow!("failed to load PHP parser: {error}"))?;
            *parser = Some(initialized);
        }

        Ok(parser
            .as_mut()
            .and_then(|parser| parser.parse(content, None)))
    })
}

#[derive(Debug, Clone)]
struct Import {
    alias: String,
    fqcn: String,
}

#[derive(Debug, Default)]
struct PhpContext {
    namespaces: Vec<(usize, String)>,
    imports: HashMap<String, String>,
    import_facts: Vec<Import>,
}

impl PhpContext {
    fn build(content: &str) -> Self {
        let namespaces = extract_namespaces(content);
        let import_facts = extract_imports(content);
        let imports = import_facts
            .iter()
            .map(|import| (import.alias.to_ascii_lowercase(), import.fqcn.clone()))
            .collect();

        Self {
            namespaces,
            imports,
            import_facts,
        }
    }

    fn namespace_at(&self, offset: usize) -> Option<&str> {
        self.namespaces
            .iter()
            .take_while(|(namespace_offset, _)| *namespace_offset <= offset)
            .last()
            .map(|(_, namespace)| namespace.as_str())
    }

    fn resolve_class_name(&self, raw_name: &str, offset: usize) -> Option<String> {
        let cleaned = clean_php_name(raw_name)?;
        let absolute = cleaned.starts_with('\\');
        let name = cleaned.trim_start_matches('\\');

        if name.eq_ignore_ascii_case("self")
            || name.eq_ignore_ascii_case("static")
            || name.eq_ignore_ascii_case("parent")
            || is_primitive_type(name)
        {
            return None;
        }

        if let Some(rest) = name.strip_prefix("namespace\\") {
            return self
                .namespace_at(offset)
                .map(|namespace| qualify_name(namespace, rest));
        }

        if absolute || is_shopware_fqcn(name) {
            return Some(name.to_string());
        }

        let mut segments = name.splitn(2, '\\');
        let first = segments.next()?;
        let rest = segments.next();

        if let Some(imported) = self.imports.get(&first.to_ascii_lowercase()) {
            return Some(match rest {
                Some(rest) => format!("{imported}\\{rest}"),
                None => imported.clone(),
            });
        }

        if let Some(namespace) = self.namespace_at(offset) {
            return Some(qualify_name(namespace, name));
        }

        Some(name.to_string())
    }
}

struct FactSink<'a> {
    content: &'a str,
    path: &'a Path,
    include_snippets: bool,
    line_index: LineIndex,
    facts: Vec<Fact>,
    seen: HashSet<(String, FactRole, usize, Option<usize>)>,
}

impl<'a> FactSink<'a> {
    fn new(content: &'a str, path: &'a Path, include_snippets: bool) -> Self {
        Self {
            content,
            path,
            include_snippets,
            line_index: LineIndex::new(content),
            facts: Vec::new(),
            seen: HashSet::new(),
        }
    }

    fn into_vec(self) -> Vec<Fact> {
        self.facts
    }

    fn usage(
        &mut self,
        surface: impl Into<String>,
        offset: usize,
        confidence: Confidence,
        usage_kind: impl Into<String>,
    ) {
        self.push(Fact::usage(
            surface,
            self.evidence(offset),
            confidence,
            usage_kind,
        ));
    }

    fn definition(
        &mut self,
        surface: impl Into<String>,
        offset: usize,
        usage_kind: impl Into<String>,
        signature: Option<Signature>,
    ) {
        self.push(Fact::definition(
            surface,
            self.evidence(offset),
            Confidence::High,
            usage_kind,
            signature,
        ));
    }

    fn push(&mut self, fact: Fact) {
        let key = (
            fact.surface.as_str().to_string(),
            fact.role,
            fact.evidence.line,
            fact.evidence.column,
        );

        if self.seen.insert(key) {
            self.facts.push(fact);
        }
    }

    fn evidence(&self, offset: usize) -> Evidence {
        let safe_offset = offset.min(self.content.len());
        let (line, column) = self.line_index.line_col(self.content, safe_offset);

        Evidence {
            path: self.path.to_path_buf(),
            line,
            column: Some(column),
            snippet: self
                .include_snippets
                .then(|| self.line_index.snippet(self.content, line))
                .flatten(),
        }
    }
}

#[derive(Debug, Clone)]
struct ClassSpan {
    kind: String,
    fqcn: String,
    name_offset: usize,
    body_start: usize,
    body_end: usize,
}

fn extract_usages(
    content: &str,
    tree: Option<&Tree>,
    context: &PhpContext,
    sink: &mut FactSink<'_>,
) {
    if let Some(tree) = tree {
        emit_ast_class_usages(content, tree.root_node(), context, sink);
        emit_ast_instance_method_usages(content, tree.root_node(), context, sink);
    }
    if content.contains("::") {
        emit_static_access_usages(content, context, sink);
    }
    if may_contain_string_literal(content) {
        emit_string_usages(content, sink);
    }
}

fn may_contain_string_literal(content: &str) -> bool {
    content
        .as_bytes()
        .iter()
        .any(|byte| matches!(byte, b'\'' | b'"'))
}

fn extract_definitions(content: &str, context: &PhpContext, sink: &mut FactSink<'_>) {
    let class_spans = extract_class_spans(content, context);

    for class_span in &class_spans {
        sink.definition(
            format!("php:class:{}", class_span.fqcn),
            class_span.name_offset,
            format!("{}-definition", class_span.kind),
            None,
        );

        extract_method_definitions(content, context, class_span, sink);
        extract_property_definitions(content, class_span, sink);
        extract_const_definitions(content, class_span, sink);
        extract_enum_case_definitions(content, class_span, sink);
    }
}

fn extract_namespaces(content: &str) -> Vec<(usize, String)> {
    namespace_re()
        .captures_iter(content)
        .filter_map(|capture| {
            let namespace = capture.get(1)?;
            Some((namespace.start(), namespace.as_str().trim().to_string()))
        })
        .collect()
}

fn extract_imports(content: &str) -> Vec<Import> {
    let header_end = first_type_declaration_offset(content).unwrap_or(content.len());
    let header = &content[..header_end];
    let mut imports = Vec::new();

    for capture in import_statement_re().captures_iter(header) {
        let Some(body) = capture.get(1) else {
            continue;
        };

        let body_text = body.as_str().trim_start();
        if body_text.starts_with('(')
            || starts_with_keyword(body_text, "function")
            || starts_with_keyword(body_text, "const")
        {
            continue;
        }

        parse_import_body(body.as_str(), &mut imports);
    }

    imports
}

fn parse_import_body(body: &str, imports: &mut Vec<Import>) {
    let trimmed = body.trim();

    if let (Some(open), Some(close)) = (trimmed.find('{'), trimmed.rfind('}'))
        && close > open
    {
        let prefix = trimmed[..open].trim().trim_end_matches('\\').to_string();
        let inner = &trimmed[open + 1..close];

        for (_, item) in split_top_level_with_offsets(inner, ',') {
            parse_import_item(item, Some(prefix.as_str()), imports);
        }

        return;
    }

    for (_, item) in split_top_level_with_offsets(body, ',') {
        parse_import_item(item, None, imports);
    }
}

fn parse_import_item(item: &str, prefix: Option<&str>, imports: &mut Vec<Import>) {
    let item = item.trim();

    if item.is_empty()
        || starts_with_keyword(item, "function")
        || starts_with_keyword(item, "const")
        || item.starts_with('(')
    {
        return;
    }

    let (name_part, alias_part) = split_alias(item);
    let Some(name) = clean_php_name(name_part) else {
        return;
    };

    let fqcn = match prefix {
        Some(prefix) if !name.starts_with('\\') => {
            let prefix = prefix
                .trim()
                .trim_start_matches('\\')
                .trim_end_matches('\\');
            format!("{prefix}\\{}", name.trim_start_matches('\\'))
        }
        _ => name.trim_start_matches('\\').to_string(),
    };

    if !is_valid_php_name(&fqcn) {
        return;
    }

    let alias = alias_part
        .and_then(clean_php_name)
        .unwrap_or_else(|| last_name_segment(&fqcn).to_string());

    imports.push(Import { alias, fqcn });
}

fn emit_ast_class_usages(
    content: &str,
    root: Node<'_>,
    context: &PhpContext,
    sink: &mut FactSink<'_>,
) {
    let mut cursor = root.walk();
    let mut stack = vec![root];
    let first_type_declaration = first_type_declaration_offset(content).unwrap_or(usize::MAX);

    while let Some(node) = stack.pop() {
        match node.kind() {
            "object_creation_expression" => {
                emit_first_direct_ast_class_reference(content, node, context, sink, "new");
            }
            "base_clause" => {
                emit_direct_ast_class_references(content, node, context, sink, "base-clause");
            }
            "named_type" => {
                emit_direct_ast_class_references(content, node, context, sink, "type-hint");
            }
            "attribute" => {
                emit_first_direct_ast_class_reference(content, node, context, sink, "attribute");
            }
            "use_declaration" if node.start_byte() > first_type_declaration => {
                emit_direct_ast_class_references(content, node, context, sink, "trait-use");
            }
            _ => {}
        }

        for child in node.children(&mut cursor) {
            if child.is_named() {
                stack.push(child);
            }
        }
    }
}

fn emit_first_direct_ast_class_reference(
    content: &str,
    node: Node<'_>,
    context: &PhpContext,
    sink: &mut FactSink<'_>,
    usage_kind: &str,
) {
    let mut cursor = node.walk();

    for child in node.named_children(&mut cursor) {
        if emit_ast_class_reference(content, child, context, sink, usage_kind) {
            break;
        }
    }
}

fn emit_direct_ast_class_references(
    content: &str,
    node: Node<'_>,
    context: &PhpContext,
    sink: &mut FactSink<'_>,
    usage_kind: &str,
) {
    let mut cursor = node.walk();

    for child in node.named_children(&mut cursor) {
        if !emit_ast_class_reference(content, child, context, sink, usage_kind)
            && matches!(child.kind(), "use_list")
        {
            emit_ast_class_reference_descendants(content, child, context, sink, usage_kind);
        }
    }
}

fn emit_ast_class_reference_descendants(
    content: &str,
    root: Node<'_>,
    context: &PhpContext,
    sink: &mut FactSink<'_>,
    usage_kind: &str,
) {
    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        if emit_ast_class_reference(content, node, context, sink, usage_kind) {
            continue;
        }

        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            stack.push(child);
        }
    }
}

fn emit_ast_class_reference(
    content: &str,
    node: Node<'_>,
    context: &PhpContext,
    sink: &mut FactSink<'_>,
    usage_kind: &str,
) -> bool {
    if !matches!(node.kind(), "name" | "qualified_name" | "relative_name") {
        return false;
    }

    if let Ok(raw_name) = node.utf8_text(content.as_bytes()) {
        emit_shopware_class_usage(context, sink, raw_name, node.start_byte(), usage_kind);
    }

    true
}

#[derive(Default)]
struct InstanceTypes {
    properties: HashMap<String, String>,
    variables: HashMap<String, String>,
}

impl InstanceTypes {
    fn is_empty(&self) -> bool {
        self.properties.is_empty() && self.variables.is_empty()
    }
}

enum InstanceReceiver {
    Property(String),
    Variable(String),
}

struct InstanceMethodCall {
    receiver: InstanceReceiver,
    method_name: String,
    method_offset: usize,
}

fn emit_ast_instance_method_usages(
    content: &str,
    root: Node<'_>,
    context: &PhpContext,
    sink: &mut FactSink<'_>,
) {
    let types = collect_instance_types(content, root, context);
    if types.is_empty() {
        return;
    }

    let mut cursor = root.walk();
    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        if matches!(
            node.kind(),
            "member_call_expression" | "nullsafe_member_call_expression"
        ) && let Some(call) = parse_instance_method_call(content, node)
        {
            let fqcn = match &call.receiver {
                InstanceReceiver::Property(property) => types.properties.get(property),
                InstanceReceiver::Variable(variable) => types.variables.get(variable),
            };

            if let Some(fqcn) = fqcn {
                sink.usage(
                    format!("php:method:{fqcn}::{}", call.method_name),
                    call.method_offset,
                    Confidence::High,
                    "instance-call",
                );
            }
        }

        for child in node.children(&mut cursor) {
            if child.is_named() {
                stack.push(child);
            }
        }
    }
}

fn collect_instance_types(content: &str, root: Node<'_>, context: &PhpContext) -> InstanceTypes {
    let mut types = InstanceTypes::default();
    let mut cursor = root.walk();
    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        match node.kind() {
            "property_declaration" => {
                if let Some(fqcn) = resolved_node_type(content, node, context) {
                    for property in property_names(content, node) {
                        types.properties.insert(property, fqcn.clone());
                    }
                }
            }
            "property_promotion_parameter" => {
                if let (Some(fqcn), Some(variable)) = (
                    resolved_node_type(content, node, context),
                    parameter_name(content, node),
                ) {
                    types.properties.insert(variable.clone(), fqcn.clone());
                    types.variables.insert(variable, fqcn);
                }
            }
            "simple_parameter" | "variadic_parameter" => {
                if let (Some(fqcn), Some(variable)) = (
                    resolved_node_type(content, node, context),
                    parameter_name(content, node),
                ) {
                    types.variables.insert(variable, fqcn);
                }
            }
            _ => {}
        }

        for child in node.children(&mut cursor) {
            if child.is_named() {
                stack.push(child);
            }
        }
    }

    let mut cursor = root.walk();
    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        if node.kind() == "assignment_expression" {
            map_assigned_property_type(content, node, &mut types);
        }

        for child in node.children(&mut cursor) {
            if child.is_named() {
                stack.push(child);
            }
        }
    }

    types
}

fn resolved_node_type(content: &str, node: Node<'_>, context: &PhpContext) -> Option<String> {
    let type_node = node.child_by_field_name("type")?;

    for named_type in descendant_kinds(type_node, "named_type") {
        if let Ok(raw_type) = named_type.utf8_text(content.as_bytes())
            && let Some(fqcn) = context.resolve_class_name(raw_type, named_type.start_byte())
            && is_shopware_fqcn(&fqcn)
        {
            return Some(fqcn);
        }
    }

    None
}

fn first_descendant_kind<'tree>(root: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    descendant_kinds(root, kind).into_iter().next()
}

fn descendant_kinds<'tree>(root: Node<'tree>, kind: &str) -> Vec<Node<'tree>> {
    let mut matches = Vec::new();
    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        if node.kind() == kind {
            matches.push(node);
            continue;
        }

        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            stack.push(child);
        }
    }

    matches
}

fn property_names(content: &str, root: Node<'_>) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = root.walk();

    for child in root.named_children(&mut cursor) {
        if child.kind() == "property_element"
            && let Some(name_node) = child.child_by_field_name("name")
            && let Ok(raw_name) = name_node.utf8_text(content.as_bytes())
            && let Some(name) = raw_name.strip_prefix('$')
        {
            names.push(name.to_string());
        }
    }

    names
}

fn parameter_name(content: &str, node: Node<'_>) -> Option<String> {
    let name_node = node.child_by_field_name("name")?;
    if name_node.kind() == "variable_name" {
        let raw_name = name_node.utf8_text(content.as_bytes()).ok()?;
        return raw_name.strip_prefix('$').map(str::to_string);
    }

    let variable = first_descendant_kind(name_node, "variable_name")?;
    let raw_name = variable.utf8_text(content.as_bytes()).ok()?;
    raw_name.strip_prefix('$').map(str::to_string)
}

fn map_assigned_property_type(content: &str, node: Node<'_>, types: &mut InstanceTypes) {
    let Some(left) = node.child_by_field_name("left") else {
        return;
    };
    let Some(right) = node.child_by_field_name("right") else {
        return;
    };
    let Some(property) = this_property_name(content, left) else {
        return;
    };
    let Some(variable) = variable_receiver_name(content, right) else {
        return;
    };
    let Some(fqcn) = types.variables.get(&variable).cloned() else {
        return;
    };

    types.properties.insert(property, fqcn);
}

fn parse_instance_method_call(content: &str, node: Node<'_>) -> Option<InstanceMethodCall> {
    let name_node = node.child_by_field_name("name")?;
    if name_node.kind() != "name" {
        return None;
    }

    let method_name = name_node.utf8_text(content.as_bytes()).ok()?.to_string();
    let object_node = node.child_by_field_name("object")?;
    let receiver = match object_node.kind() {
        "member_access_expression" | "nullsafe_member_access_expression" => {
            InstanceReceiver::Property(this_property_name(content, object_node)?)
        }
        "variable_name" => {
            InstanceReceiver::Variable(variable_receiver_name(content, object_node)?)
        }
        _ => return None,
    };

    Some(InstanceMethodCall {
        receiver,
        method_name,
        method_offset: name_node.start_byte(),
    })
}

fn this_property_name(content: &str, node: Node<'_>) -> Option<String> {
    let name_node = node.child_by_field_name("name")?;
    if name_node.kind() != "name" {
        return None;
    }

    let object_node = node.child_by_field_name("object")?;
    let object_name = object_node.utf8_text(content.as_bytes()).ok()?;
    if object_name != "$this" {
        return None;
    }

    Some(name_node.utf8_text(content.as_bytes()).ok()?.to_string())
}

fn variable_receiver_name(content: &str, node: Node<'_>) -> Option<String> {
    if node.kind() != "variable_name" {
        return None;
    }

    node.utf8_text(content.as_bytes())
        .ok()?
        .strip_prefix('$')
        .map(str::to_string)
}

fn emit_static_access_usages(content: &str, context: &PhpContext, sink: &mut FactSink<'_>) {
    for capture in static_access_re().captures_iter(content) {
        let Some(scope) = capture.get(1) else {
            continue;
        };
        let Some(member) = capture.get(2) else {
            continue;
        };

        let Some(fqcn) = context.resolve_class_name(scope.as_str(), scope.start()) else {
            continue;
        };

        if !is_shopware_fqcn(&fqcn) {
            continue;
        }

        let member_name = member.as_str();

        if member_name.eq_ignore_ascii_case("class") {
            sink.usage(
                format!("php:class:{fqcn}"),
                scope.start(),
                Confidence::High,
                "class-constant",
            );
        } else if looks_like_constant_name(member_name) {
            sink.usage(
                format!("php:class:{fqcn}"),
                scope.start(),
                Confidence::High,
                "class-constant",
            );
            sink.usage(
                format!("php:const:{fqcn}::{member_name}"),
                member.start(),
                Confidence::High,
                "class-constant",
            );
        } else {
            sink.usage(
                format!("php:class:{fqcn}"),
                scope.start(),
                Confidence::High,
                "static-call",
            );
            sink.usage(
                format!("php:method:{fqcn}::{member_name}"),
                member.start(),
                Confidence::High,
                "static-call",
            );
        }
    }
}

fn emit_string_usages(content: &str, sink: &mut FactSink<'_>) {
    for literal in string_literals(content) {
        let value = literal.value.trim();
        if value.is_empty() || value.contains(char::is_whitespace) {
            continue;
        }

        let value_offset = literal.value_offset;
        let route_context = preceding_context(content, literal.start, 48);

        if looks_like_route_name(value) || looks_like_route_attribute_name(value, &route_context) {
            sink.usage(
                format!("route:name:{value}"),
                value_offset,
                Confidence::Medium,
                "route-string",
            );
            continue;
        }

        if let Some(entity) = repository_entity_name(value) {
            sink.usage(
                format!("service:id:{value}"),
                value_offset,
                Confidence::Medium,
                "repository-service-string",
            );
            sink.usage(
                format!("dal:entity:{entity}"),
                value_offset,
                Confidence::Medium,
                "repository-service-string",
            );
            continue;
        }

        if looks_like_service_id(value) {
            sink.usage(
                format!("service:id:{value}"),
                value_offset,
                Confidence::Medium,
                "service-string",
            );
            continue;
        }

        if looks_like_dal_entity(value) {
            sink.usage(
                format!("dal:entity:{value}"),
                value_offset,
                Confidence::Medium,
                "entity-string",
            );
        }
    }
}

fn emit_shopware_class_usage(
    context: &PhpContext,
    sink: &mut FactSink<'_>,
    raw_name: &str,
    offset: usize,
    usage_kind: &str,
) {
    let Some(fqcn) = context.resolve_class_name(raw_name, offset) else {
        return;
    };

    if is_shopware_fqcn(&fqcn) {
        sink.usage(
            format!("php:class:{fqcn}"),
            offset,
            Confidence::High,
            usage_kind,
        );
    }
}

fn extract_class_spans(content: &str, context: &PhpContext) -> Vec<ClassSpan> {
    let mut spans = Vec::new();

    for capture in class_declaration_re().captures_iter(content) {
        let Some(kind) = capture.get(1) else {
            continue;
        };
        let Some(name) = capture.get(2) else {
            continue;
        };

        let Some(brace_pos) = content[name.end()..]
            .find('{')
            .map(|offset| name.end() + offset)
        else {
            continue;
        };

        let Some(body_end) = find_matching_delimiter(content, brace_pos, b'{', b'}') else {
            continue;
        };

        let namespace = context.namespace_at(capture.get(0).unwrap().start());
        let fqcn = match namespace {
            Some(namespace) if !namespace.is_empty() => qualify_name(namespace, name.as_str()),
            _ => name.as_str().to_string(),
        };

        spans.push(ClassSpan {
            kind: kind.as_str().to_string(),
            fqcn,
            name_offset: name.start(),
            body_start: brace_pos + 1,
            body_end,
        });
    }

    spans
}

fn extract_method_definitions(
    content: &str,
    context: &PhpContext,
    class_span: &ClassSpan,
    sink: &mut FactSink<'_>,
) {
    let body = &content[class_span.body_start..class_span.body_end];

    for capture in method_declaration_re().captures_iter(body) {
        let Some(name) = capture.get(1) else {
            continue;
        };

        let method_start = class_span.body_start + capture.get(0).unwrap().start();
        if !is_direct_class_member(content, class_span.body_start, method_start) {
            continue;
        }

        let name_start = class_span.body_start + name.start();
        let name_end = class_span.body_start + name.end();

        let Some(open_paren) = content[name_end..class_span.body_end]
            .find('(')
            .map(|offset| name_end + offset)
        else {
            continue;
        };

        let Some(close_paren) = find_matching_delimiter(content, open_paren, b'(', b')') else {
            continue;
        };

        let params = &content[open_paren + 1..close_paren];
        let parameters = parse_parameters(
            params,
            open_paren + 1,
            context,
            Some(class_span.fqcn.as_str()),
        );
        let return_type = return_type_after(content, close_paren).map(|(return_type, offset)| {
            normalize_signature_type(
                &return_type,
                context,
                offset,
                Some(class_span.fqcn.as_str()),
            )
        });

        let declaration = capture.get(0).unwrap().as_str();
        let signature = Signature {
            visibility: Some(method_visibility(declaration).to_string()),
            parameters,
            return_type,
        };

        sink.definition(
            format!("php:method:{}::{}", class_span.fqcn, name.as_str()),
            name_start,
            "method-definition",
            Some(signature),
        );
    }
}

fn extract_property_definitions(content: &str, class_span: &ClassSpan, sink: &mut FactSink<'_>) {
    let body = &content[class_span.body_start..class_span.body_end];

    for declaration in property_declaration_re().find_iter(body) {
        if declaration.as_str().contains("function") {
            continue;
        }

        let declaration_start = class_span.body_start + declaration.start();
        if !is_direct_class_member(content, class_span.body_start, declaration_start) {
            continue;
        }

        for property in property_var_re().find_iter(declaration.as_str()) {
            let name = property.as_str();
            sink.definition(
                format!("php:property:{}::{name}", class_span.fqcn),
                declaration_start + property.start(),
                "property-definition",
                None,
            );
        }
    }
}

fn extract_const_definitions(content: &str, class_span: &ClassSpan, sink: &mut FactSink<'_>) {
    let body = &content[class_span.body_start..class_span.body_end];

    for capture in const_declaration_re().captures_iter(body) {
        let declaration_start = class_span.body_start + capture.get(0).unwrap().start();
        if !is_direct_class_member(content, class_span.body_start, declaration_start) {
            continue;
        }

        let Some(body) = capture.get(1) else {
            continue;
        };

        for const_name in const_name_re().captures_iter(body.as_str()) {
            let Some(name) = const_name.get(1) else {
                continue;
            };

            sink.definition(
                format!("php:const:{}::{}", class_span.fqcn, name.as_str()),
                class_span.body_start + body.start() + name.start(),
                "const-definition",
                None,
            );
        }
    }
}

fn extract_enum_case_definitions(content: &str, class_span: &ClassSpan, sink: &mut FactSink<'_>) {
    if class_span.kind != "enum" {
        return;
    }

    let body = &content[class_span.body_start..class_span.body_end];

    for capture in enum_case_re().captures_iter(body) {
        let case_start = class_span.body_start + capture.get(0).unwrap().start();
        if !is_direct_class_member(content, class_span.body_start, case_start) {
            continue;
        }

        let Some(name) = capture.get(1) else {
            continue;
        };

        sink.definition(
            format!("php:const:{}::{}", class_span.fqcn, name.as_str()),
            class_span.body_start + name.start(),
            "enum-case-definition",
            None,
        );
    }
}

fn parse_parameters(
    params: &str,
    params_offset: usize,
    context: &PhpContext,
    current_class: Option<&str>,
) -> Vec<Parameter> {
    split_top_level_with_offsets(params, ',')
        .into_iter()
        .filter_map(|(offset, parameter)| {
            let global_offset = params_offset + offset;
            parse_parameter(parameter, global_offset, context, current_class)
        })
        .collect()
}

fn parse_parameter(
    parameter: &str,
    parameter_offset: usize,
    context: &PhpContext,
    current_class: Option<&str>,
) -> Option<Parameter> {
    let variable = parameter_variable_re().find(parameter)?;
    let name = variable.as_str().trim_start_matches('$').to_string();
    let before_variable = &parameter[..variable.start()];
    let type_name = parameter_type_part(parameter, parameter_offset).map(|(raw_type, offset)| {
        normalize_signature_type(&raw_type, context, offset, current_class)
    });
    let required = !parameter[variable.end()..].contains('=');

    if before_variable.contains("function") {
        return None;
    }

    Some(Parameter {
        name,
        type_name,
        required,
    })
}

fn parameter_type_part(parameter: &str, parameter_offset: usize) -> Option<(String, usize)> {
    let variable = parameter_variable_re().find(parameter)?;
    let before_variable = parameter[..variable.start()].trim();

    if before_variable.is_empty() {
        return None;
    }

    let mut type_parts = Vec::new();
    let mut type_start = None;

    for token in before_variable.split_whitespace() {
        let cleaned = token
            .trim_matches('&')
            .trim_start_matches("...")
            .trim_matches(',');

        if cleaned.is_empty()
            || matches!(
                cleaned,
                "public" | "protected" | "private" | "readonly" | "static" | "final"
            )
            || cleaned.starts_with("#[")
        {
            continue;
        }

        if type_start.is_none()
            && let Some(position) = parameter.find(token)
        {
            type_start = Some(parameter_offset + position);
        }

        type_parts.push(cleaned);
    }

    if type_parts.is_empty() {
        None
    } else {
        Some((type_parts.join(" "), type_start.unwrap_or(parameter_offset)))
    }
}

fn return_type_after(content: &str, close_paren: usize) -> Option<(String, usize)> {
    let mut index = close_paren + 1;
    let bytes = content.as_bytes();

    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }

    if bytes.get(index) != Some(&b':') {
        return None;
    }

    index += 1;
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }

    let start = index;
    let mut depth = 0usize;

    while index < bytes.len() {
        match bytes[index] {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth = depth.saturating_sub(1),
            b'{' | b';' if depth == 0 => break,
            _ => {}
        }
        index += 1;
    }

    let return_type = content[start..index].trim();
    if return_type.is_empty() {
        None
    } else {
        Some((return_type.to_string(), start))
    }
}

fn normalize_signature_type(
    type_name: &str,
    context: &PhpContext,
    offset: usize,
    current_class: Option<&str>,
) -> String {
    let mut normalized = String::new();
    let mut part_start = 0usize;

    for (index, ch) in type_name.char_indices() {
        if ch == '|' || ch == '&' {
            normalized.push_str(&normalize_single_type(
                &type_name[part_start..index],
                context,
                offset + part_start,
                current_class,
            ));
            normalized.push(ch);
            part_start = index + ch.len_utf8();
        }
    }

    normalized.push_str(&normalize_single_type(
        &type_name[part_start..],
        context,
        offset + part_start,
        current_class,
    ));
    normalized
}

fn normalize_single_type(
    type_part: &str,
    context: &PhpContext,
    offset: usize,
    current_class: Option<&str>,
) -> String {
    let trimmed = type_part.trim();

    if trimmed.is_empty() {
        return String::new();
    }

    let nullable = trimmed.starts_with('?');
    let name = trimmed.trim_start_matches('?').trim();
    let prefix = if nullable { "?" } else { "" };
    let lower = name.to_ascii_lowercase();

    if lower == "self" {
        return current_class
            .map(|class_name| format!("{prefix}{class_name}"))
            .unwrap_or_else(|| format!("{prefix}{lower}"));
    }

    if is_primitive_type(name) || lower == "static" || lower == "parent" {
        return format!("{prefix}{lower}");
    }

    match context.resolve_class_name(name, offset) {
        Some(resolved) => format!("{prefix}{resolved}"),
        None => format!("{prefix}{name}"),
    }
}

fn split_alias(item: &str) -> (&str, Option<&str>) {
    if let Some(alias_match) = alias_re().find(item) {
        (
            item[..alias_match.start()].trim(),
            Some(item[alias_match.end()..].trim()),
        )
    } else {
        (item.trim(), None)
    }
}

fn split_top_level_with_offsets(input: &str, separator: char) -> Vec<(usize, &str)> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut string_quote = None;
    let mut escaped = false;

    for (index, ch) in input.char_indices() {
        if let Some(quote) = string_quote {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == quote {
                string_quote = None;
            }
            continue;
        }

        match ch {
            '\'' | '"' => string_quote = Some(ch),
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            '{' => brace_depth += 1,
            '}' => brace_depth = brace_depth.saturating_sub(1),
            ch if ch == separator && paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 => {
                let part = &input[start..index];
                let trim_start = leading_whitespace_len(part);
                parts.push((start + trim_start, part.trim()));
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }

    let part = &input[start..];
    let trim_start = leading_whitespace_len(part);
    let trimmed = part.trim();
    if !trimmed.is_empty() {
        parts.push((start + trim_start, trimmed));
    }

    parts
}

fn string_literals(content: &str) -> Vec<StringLiteral> {
    let mut literals = Vec::new();
    let bytes = content.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        match bytes[index] {
            b'\'' | b'"' => {
                let quote = bytes[index];
                let start = index;
                index += 1;
                let value_start = index;
                let mut value = String::new();
                let mut escaped = false;

                while index < bytes.len() {
                    let byte = bytes[index];
                    if escaped {
                        value.push(byte as char);
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
                        break;
                    }

                    value.push(byte as char);
                    index += 1;
                }

                literals.push(StringLiteral {
                    start,
                    value_offset: value_start,
                    value,
                });
            }
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index += 2;
                while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/')
                {
                    index += 1;
                }
                index = (index + 2).min(bytes.len());
            }
            b'#' if bytes.get(index + 1) != Some(&b'[') => {
                index += 1;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            _ => index += 1,
        }
    }

    literals
}

#[derive(Debug)]
struct StringLiteral {
    start: usize,
    value_offset: usize,
    value: String,
}

fn find_matching_delimiter(content: &str, open_pos: usize, open: u8, close: u8) -> Option<usize> {
    let bytes = content.as_bytes();
    if bytes.get(open_pos) != Some(&open) {
        return None;
    }

    let mut index = open_pos;
    let mut depth = 0usize;

    while index < bytes.len() {
        match bytes[index] {
            b'\'' | b'"' => index = skip_quoted(bytes, index),
            b'/' if bytes.get(index + 1) == Some(&b'/') => index = skip_line_comment(bytes, index),
            b'/' if bytes.get(index + 1) == Some(&b'*') => index = skip_block_comment(bytes, index),
            b'#' => index = skip_line_comment(bytes, index),
            byte if byte == open => {
                depth += 1;
                index += 1;
            }
            byte if byte == close => {
                depth = depth.saturating_sub(1);
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

fn is_direct_class_member(content: &str, class_body_start: usize, offset: usize) -> bool {
    let bytes = content.as_bytes();
    let mut index = class_body_start;
    let mut depth = 0usize;

    while index < offset.min(bytes.len()) {
        match bytes[index] {
            b'\'' | b'"' => index = skip_quoted(bytes, index),
            b'/' if bytes.get(index + 1) == Some(&b'/') => index = skip_line_comment(bytes, index),
            b'/' if bytes.get(index + 1) == Some(&b'*') => index = skip_block_comment(bytes, index),
            b'#' => index = skip_line_comment(bytes, index),
            b'{' => {
                depth += 1;
                index += 1;
            }
            b'}' => {
                depth = depth.saturating_sub(1);
                index += 1;
            }
            _ => index += 1,
        }
    }

    depth == 0
}

fn skip_quoted(bytes: &[u8], start: usize) -> usize {
    let quote = bytes[start];
    let mut index = start + 1;
    let mut escaped = false;

    while index < bytes.len() {
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }

        if bytes[index] == b'\\' {
            escaped = true;
            index += 1;
            continue;
        }

        if bytes[index] == quote {
            return index + 1;
        }

        index += 1;
    }

    index
}

fn skip_line_comment(bytes: &[u8], start: usize) -> usize {
    let mut index = start + 1;
    while index < bytes.len() && bytes[index] != b'\n' {
        index += 1;
    }
    index
}

fn skip_block_comment(bytes: &[u8], start: usize) -> usize {
    let mut index = start + 2;
    while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/') {
        index += 1;
    }
    (index + 2).min(bytes.len())
}

fn first_type_declaration_offset(content: &str) -> Option<usize> {
    first_type_declaration_re()
        .find(content)
        .map(|found| found.start())
}

fn clean_php_name(raw_name: &str) -> Option<String> {
    let name = raw_name
        .trim()
        .trim_matches(',')
        .trim_matches(';')
        .trim_matches('(')
        .trim_matches(')')
        .trim();

    if name.is_empty() {
        return None;
    }

    let name = name.split("::").next().unwrap_or(name).trim();
    if is_valid_php_name(name.trim_start_matches('\\')) {
        Some(name.to_string())
    } else {
        None
    }
}

fn is_valid_php_name(name: &str) -> bool {
    !name.is_empty()
        && name.split('\\').all(|segment| {
            let mut chars = segment.chars();
            matches!(chars.next(), Some(first) if first == '_' || first.is_ascii_alphabetic())
                && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        })
}

fn qualify_name(namespace: &str, name: &str) -> String {
    if namespace.is_empty() {
        name.trim_start_matches('\\').to_string()
    } else {
        format!(
            "{}\\{}",
            namespace.trim_end_matches('\\'),
            name.trim_start_matches('\\')
        )
    }
}

fn last_name_segment(name: &str) -> &str {
    name.rsplit('\\').next().unwrap_or(name)
}

fn is_shopware_fqcn(name: &str) -> bool {
    name.starts_with("Shopware\\")
}

fn is_primitive_type(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "array"
            | "bool"
            | "boolean"
            | "callable"
            | "false"
            | "float"
            | "int"
            | "integer"
            | "iterable"
            | "mixed"
            | "never"
            | "null"
            | "object"
            | "resource"
            | "string"
            | "true"
            | "void"
    )
}

fn starts_with_keyword(input: &str, keyword: &str) -> bool {
    let input = input.trim_start();
    input
        .get(..keyword.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(keyword))
        && input[keyword.len()..]
            .chars()
            .next()
            .is_none_or(|ch| !is_identifier_char(ch))
}

fn is_identifier_char(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn leading_whitespace_len(input: &str) -> usize {
    input.len() - input.trim_start().len()
}

fn looks_like_constant_name(name: &str) -> bool {
    name.chars()
        .all(|ch| ch == '_' || ch.is_ascii_uppercase() || ch.is_ascii_digit())
}

fn method_visibility(declaration: &str) -> &'static str {
    if declaration.contains("private") {
        "private"
    } else if declaration.contains("protected") {
        "protected"
    } else {
        "public"
    }
}

fn preceding_context(content: &str, offset: usize, len: usize) -> String {
    let end = previous_char_boundary(content, offset.min(content.len()));
    let start = previous_char_boundary(content, end.saturating_sub(len));
    content[start..end].to_ascii_lowercase()
}

fn previous_char_boundary(content: &str, mut offset: usize) -> usize {
    offset = offset.min(content.len());

    while offset > 0 && !content.is_char_boundary(offset) {
        offset -= 1;
    }

    offset
}

fn looks_like_route_name(value: &str) -> bool {
    let dot_count = value.chars().filter(|ch| *ch == '.').count();
    dot_count >= 1
        && matches!(
            value.split('.').next(),
            Some("frontend" | "widgets" | "store-api" | "storefront" | "api")
        )
        && value
            .chars()
            .all(|ch| ch == '.' || ch == '-' || ch == '_' || ch.is_ascii_alphanumeric())
}

fn looks_like_route_attribute_name(value: &str, context: &str) -> bool {
    value.contains('.')
        && value
            .chars()
            .all(|ch| ch == '.' || ch == '-' || ch == '_' || ch.is_ascii_alphanumeric())
        && (context.contains("name:")
            || context.contains("'_route'")
            || context.contains("\"_route\""))
}

fn looks_like_service_id(value: &str) -> bool {
    if !value.contains('.')
        || looks_like_route_name(value)
        || !value
            .chars()
            .all(|ch| ch == '.' || ch == '-' || ch == '_' || ch.is_ascii_alphanumeric())
    {
        return false;
    }

    let lower = value.to_ascii_lowercase();
    lower.starts_with("shopware.")
        || lower.starts_with("swag.")
        || lower.ends_with(".repository")
        || lower.contains(".service")
        || lower.contains(".subscriber")
        || lower.contains(".processor")
        || lower.contains(".handler")
        || lower.contains(".factory")
        || lower.contains(".loader")
        || lower.contains(".definition")
        || lower.contains(".validator")
        || lower.contains(".gateway")
        || lower.contains(".decorator")
}

fn repository_entity_name(value: &str) -> Option<String> {
    let lower = value.to_ascii_lowercase();

    lower
        .strip_suffix(".repository")
        .or_else(|| lower.strip_suffix("_repository"))
        .filter(|entity| !entity.is_empty())
        .map(|entity| entity.replace('.', "_"))
}

fn looks_like_dal_entity(value: &str) -> bool {
    matches!(
        value,
        "category"
            | "cms_page"
            | "country"
            | "currency"
            | "customer"
            | "customer_group"
            | "language"
            | "media"
            | "order"
            | "order_line_item"
            | "payment_method"
            | "product"
            | "product_manufacturer"
            | "promotion"
            | "rule"
            | "sales_channel"
            | "shipping_method"
            | "tax"
    )
}

fn namespace_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?m)^\s*namespace\s+([A-Za-z_][A-Za-z0-9_\\]*)\s*[;{]").unwrap())
}

fn import_statement_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?ms)^\s*use\s+([^;]+);").unwrap())
}

fn alias_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\s+as\s+").unwrap())
}

fn first_type_declaration_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?m)^\s*(?:(?:abstract|final|readonly)\s+)*(?:class|interface|trait|enum)\s+")
            .unwrap()
    })
}

fn class_declaration_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?m)^\s*(?:(?:abstract|final|readonly)\s+)*(class|interface|trait|enum)\s+([A-Za-z_][A-Za-z0-9_]*)",
        )
        .unwrap()
    })
}

fn static_access_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(\\?[A-Z_][A-Za-z0-9_]*(?:\\[A-Za-z_][A-Za-z0-9_]*)*)\s*::\s*([A-Za-z_][A-Za-z0-9_]*)",
        )
        .unwrap()
    })
}

fn method_declaration_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?m)^\s*(?:(?:public|protected|private|static|abstract|final|readonly)\s+)*function\s+&?\s*([A-Za-z_][A-Za-z0-9_]*)\s*\(",
        )
        .unwrap()
    })
}

fn property_declaration_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?m)^\s*(?:(?:public|protected|private|static|readonly|var)\s+)[^;\n{}]*\$[A-Za-z_][A-Za-z0-9_]*[^;]*;")
            .unwrap()
    })
}

fn property_var_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\$[A-Za-z_][A-Za-z0-9_]*").unwrap())
}

fn const_declaration_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?m)^\s*(?:(?:public|protected|private|final)\s+)*const\s+([^;]+);").unwrap()
    })
}

fn const_name_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*)\b\s*=").unwrap())
}

fn enum_case_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?m)^\s*case\s+([A-Za-z_][A-Za-z0-9_]*)\b").unwrap())
}

fn parameter_variable_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\$[A-Za-z_][A-Za-z0-9_]*").unwrap())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::model::{Confidence, FactRole};

    use super::extract;

    fn usage_facts(content: &str) -> Vec<crate::model::Fact> {
        extract(content, Path::new("src/Test.php"), FactRole::Usage, true).unwrap()
    }

    fn definition_facts(content: &str) -> Vec<crate::model::Fact> {
        extract(
            content,
            Path::new("src/Test.php"),
            FactRole::Definition,
            false,
        )
        .unwrap()
    }

    fn has_surface(facts: &[crate::model::Fact], surface: &str) -> bool {
        facts.iter().any(|fact| fact.surface.as_str() == surface)
    }

    #[test]
    fn resolves_shopware_imports_in_type_hints() {
        let facts = usage_facts(
            r#"<?php
namespace Swag\Demo;

use Shopware\Core\Checkout\Cart\CartService;

final class Demo
{
    public function __construct(private CartService $cartService)
    {
    }
}
"#,
        );

        assert!(has_surface(
            &facts,
            "php:class:Shopware\\Core\\Checkout\\Cart\\CartService"
        ));
        assert!(
            facts
                .iter()
                .any(|fact| fact.usage_kind == "type-hint" && fact.confidence == Confidence::High)
        );
    }

    #[test]
    fn resolves_aliases_and_static_constants() {
        let facts = usage_facts(
            r#"<?php
namespace Swag\Demo;

use Shopware\Core\Content\Product\ProductEvents as Events;

final class Demo
{
    public function test(): void
    {
        $event = Events::PRODUCT_LOADED_EVENT;
    }
}
"#,
        );

        assert!(has_surface(
            &facts,
            "php:class:Shopware\\Core\\Content\\Product\\ProductEvents"
        ));
        assert!(has_surface(
            &facts,
            "php:const:Shopware\\Core\\Content\\Product\\ProductEvents::PRODUCT_LOADED_EVENT"
        ));
    }

    #[test]
    fn extracts_static_method_calls() {
        let facts = usage_facts(
            r#"<?php
namespace Swag\Demo;

use Shopware\Core\Framework\Uuid\Uuid;

final class Demo
{
    public function test(): string
    {
        return Uuid::randomHex();
    }
}
"#,
        );

        assert!(has_surface(
            &facts,
            "php:method:Shopware\\Core\\Framework\\Uuid\\Uuid::randomHex"
        ));
    }

    #[test]
    fn extracts_instance_method_calls_from_promoted_properties() {
        let facts = usage_facts(
            r#"<?php
namespace Swag\Demo;

use Shopware\Core\Checkout\Document\Service\DocumentGenerator;

final class Demo
{
    public function __construct(private readonly DocumentGenerator $documentGenerator)
    {
    }

    public function test(): void
    {
        $this->documentGenerator->generate([], []);
    }
}
"#,
        );

        assert!(has_surface(
            &facts,
            "php:method:Shopware\\Core\\Checkout\\Document\\Service\\DocumentGenerator::generate"
        ));
    }

    #[test]
    fn extracts_instance_method_calls_from_typed_properties_and_assignments() {
        let facts = usage_facts(
            r#"<?php
namespace Swag\Demo;

use Shopware\Core\Checkout\Document\Service\DocumentGenerator;

final class Demo
{
    private DocumentGenerator $documentGenerator;

    private $fallbackGenerator;

    public function __construct(DocumentGenerator $documentGenerator)
    {
        $this->documentGenerator = $documentGenerator;
        $this->fallbackGenerator = $documentGenerator;
    }

    public function test(DocumentGenerator $documentGenerator): void
    {
        $this->documentGenerator->generate([], []);
        $this->fallbackGenerator->generate([], []);
        $documentGenerator->generate([], []);
    }
}
"#,
        );

        let matches = facts
            .iter()
            .filter(|fact| {
                fact.surface.as_str()
                    == "php:method:Shopware\\Core\\Checkout\\Document\\Service\\DocumentGenerator::generate"
                    && fact.usage_kind == "instance-call"
            })
            .count();

        assert_eq!(matches, 3);
    }

    #[test]
    fn extracts_service_strings_and_repository_entities() {
        let facts = usage_facts(
            r#"<?php
final class Demo
{
    public function test(): void
    {
        $this->container->get('cart.processor');
        $this->container->get('product.repository');
    }
}
"#,
        );

        assert!(has_surface(&facts, "service:id:cart.processor"));
        assert!(has_surface(&facts, "service:id:product.repository"));
        assert!(has_surface(&facts, "dal:entity:product"));
    }

    #[test]
    fn extracts_route_strings() {
        let facts = usage_facts(
            r#"<?php
use Symfony\Component\Routing\Attribute\Route;

#[Route(path: '/demo', name: 'frontend.demo.page')]
final class Demo
{
}
"#,
        );

        assert!(has_surface(&facts, "route:name:frontend.demo.page"));
    }

    #[test]
    fn extracts_strings_after_non_ascii_context_without_panicking() {
        let facts = usage_facts(
            r#"<?php

final class Demo
{
    public function test(): void
    {
        $label = 'für den Warenkorb';
        $route = ['name' => 'frontend.demo.page'];
    }
}
"#,
        );

        assert!(has_surface(&facts, "route:name:frontend.demo.page"));
    }

    #[test]
    fn extracts_definition_method_signatures() {
        let facts = definition_facts(
            r#"<?php
namespace Shopware\Core\Checkout\Cart;

use Shopware\Core\Framework\Context;

final class CartService
{
    public const NAME = 'cart';

    private Context $context;

    public function recalculate(Context $context, ?string $token = null): void
    {
    }
}
"#,
        );

        assert!(has_surface(
            &facts,
            "php:class:Shopware\\Core\\Checkout\\Cart\\CartService"
        ));
        assert!(has_surface(
            &facts,
            "php:property:Shopware\\Core\\Checkout\\Cart\\CartService::$context"
        ));
        assert!(has_surface(
            &facts,
            "php:const:Shopware\\Core\\Checkout\\Cart\\CartService::NAME"
        ));

        let method = facts
            .iter()
            .find(|fact| {
                fact.surface.as_str()
                    == "php:method:Shopware\\Core\\Checkout\\Cart\\CartService::recalculate"
            })
            .expect("method fact");
        let signature = method.signature.as_ref().expect("signature");

        assert_eq!(signature.visibility.as_deref(), Some("public"));
        assert_eq!(signature.return_type.as_deref(), Some("void"));
        assert_eq!(signature.parameters.len(), 2);
        assert_eq!(signature.parameters[0].name, "context");
        assert_eq!(
            signature.parameters[0].type_name.as_deref(),
            Some("Shopware\\Core\\Framework\\Context")
        );
        assert!(signature.parameters[0].required);
        assert_eq!(signature.parameters[1].name, "token");
        assert_eq!(
            signature.parameters[1].type_name.as_deref(),
            Some("?string")
        );
        assert!(!signature.parameters[1].required);
    }
}
