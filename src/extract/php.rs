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

#[derive(Debug, Clone)]
struct TypeInfo {
    fqcn: String,
    confidence: Confidence,
}

impl TypeInfo {
    fn new(fqcn: String, confidence: Confidence) -> Option<Self> {
        is_shopware_fqcn(&fqcn).then_some(Self { fqcn, confidence })
    }

    fn with_max_confidence(&self, confidence: Confidence) -> Self {
        Self {
            fqcn: self.fqcn.clone(),
            confidence: self.confidence.min(confidence),
        }
    }
}

#[derive(Default)]
struct PhpSemanticIndex<'tree> {
    properties: HashMap<String, TypeInfo>,
    routines: HashMap<String, RoutineSummary<'tree>>,
}

#[derive(Clone)]
struct RoutineSummary<'tree> {
    parameters: Vec<String>,
    param_types: HashMap<String, TypeInfo>,
    return_type: Option<TypeInfo>,
    body: Option<Node<'tree>>,
    param_method_calls: Vec<ParamMethodCall>,
}

#[derive(Clone)]
struct ParamMethodCall {
    param_index: usize,
    method_name: String,
}

#[derive(Default)]
struct TypeEnv {
    variables: HashMap<String, TypeInfo>,
    properties: HashMap<String, TypeInfo>,
}

impl TypeEnv {
    fn for_routine(index: &PhpSemanticIndex<'_>, routine: &RoutineSummary<'_>) -> Self {
        Self {
            variables: routine.param_types.clone(),
            properties: index.properties.clone(),
        }
    }

    fn top_level(index: &PhpSemanticIndex<'_>) -> Self {
        Self {
            variables: HashMap::new(),
            properties: index.properties.clone(),
        }
    }
}

struct CallParts<'tree> {
    name: String,
    name_offset: usize,
    arguments: Vec<Node<'tree>>,
}

fn emit_ast_instance_method_usages(
    content: &str,
    root: Node<'_>,
    context: &PhpContext,
    sink: &mut FactSink<'_>,
) {
    if !content.contains("->") {
        return;
    }

    let mut index = collect_php_semantic_index(content, root, context);

    infer_assigned_property_types(content, context, &mut index);
    infer_routine_summaries(content, context, &mut index);
    infer_assigned_property_types(content, context, &mut index);

    for routine in index.routines.values() {
        let Some(body) = routine.body else {
            continue;
        };
        let mut env = TypeEnv::for_routine(&index, routine);
        emit_semantic_usages_in_node(content, body, &mut env, &index, context, sink);
    }

    let mut env = TypeEnv::top_level(&index);
    emit_semantic_usages_in_node(content, root, &mut env, &index, context, sink);
}

fn collect_php_semantic_index<'tree>(
    content: &str,
    root: Node<'tree>,
    context: &PhpContext,
) -> PhpSemanticIndex<'tree> {
    let mut index = PhpSemanticIndex::default();
    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        match node.kind() {
            "property_declaration" => {
                if let Some(type_info) = resolved_node_type(content, node, context) {
                    for property in property_names(content, node) {
                        index.properties.insert(property, type_info.clone());
                    }
                }
            }
            "property_promotion_parameter" => {
                if let (Some(type_info), Some(parameter)) = (
                    resolved_node_type(content, node, context),
                    parameter_name(content, node),
                ) {
                    index.properties.insert(parameter, type_info);
                }
            }
            "method_declaration" | "function_definition" => {
                if let Some((name, summary)) = routine_summary(content, node, context) {
                    for (property, type_info) in promoted_property_types(content, node, context) {
                        index.properties.insert(property, type_info);
                    }
                    index.routines.insert(routine_key(&name), summary);
                }
            }
            _ => {}
        }

        for child in named_children(node) {
            if !matches!(node.kind(), "method_declaration" | "function_definition") {
                stack.push(child);
            }
        }
    }

    index
}

fn promoted_property_types(
    content: &str,
    node: Node<'_>,
    context: &PhpContext,
) -> Vec<(String, TypeInfo)> {
    let Some(parameters_node) = node.child_by_field_name("parameters") else {
        return Vec::new();
    };

    named_children(parameters_node)
        .into_iter()
        .filter(|parameter| parameter.kind() == "property_promotion_parameter")
        .filter_map(|parameter| {
            Some((
                parameter_name(content, parameter)?,
                resolved_node_type(content, parameter, context)?,
            ))
        })
        .collect()
}

fn routine_summary<'tree>(
    content: &str,
    node: Node<'tree>,
    context: &PhpContext,
) -> Option<(String, RoutineSummary<'tree>)> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(content.as_bytes()).ok()?.to_string();
    let mut parameters = Vec::new();
    let mut param_types = HashMap::new();

    if let Some(parameters_node) = node.child_by_field_name("parameters") {
        for parameter in named_children(parameters_node) {
            if !matches!(
                parameter.kind(),
                "simple_parameter" | "property_promotion_parameter" | "variadic_parameter"
            ) {
                continue;
            }

            let Some(parameter_name) = parameter_name(content, parameter) else {
                continue;
            };

            if let Some(type_info) = resolved_node_type(content, parameter, context) {
                param_types.insert(parameter_name.clone(), type_info);
            }

            parameters.push(parameter_name);
        }
    }

    let return_type = node
        .child_by_field_name("return_type")
        .and_then(|return_type| {
            resolved_type_node(content, return_type, context, Confidence::High)
        });

    Some((
        name,
        RoutineSummary {
            parameters,
            param_types,
            return_type,
            body: node.child_by_field_name("body"),
            param_method_calls: Vec::new(),
        },
    ))
}

fn infer_assigned_property_types(
    content: &str,
    context: &PhpContext,
    index: &mut PhpSemanticIndex<'_>,
) {
    let mut updates = Vec::new();

    for routine in index.routines.values() {
        let Some(body) = routine.body else {
            continue;
        };
        let mut env = TypeEnv::for_routine(index, routine);
        collect_assigned_property_types(content, body, &mut env, index, context, &mut updates);
    }

    for (property, type_info) in updates {
        index.properties.insert(property, type_info);
    }
}

fn collect_assigned_property_types(
    content: &str,
    node: Node<'_>,
    env: &mut TypeEnv,
    index: &PhpSemanticIndex<'_>,
    context: &PhpContext,
    updates: &mut Vec<(String, TypeInfo)>,
) {
    match node.kind() {
        "method_declaration" | "function_definition" => {}
        "assignment_expression" => {
            if let Some(right) = node.child_by_field_name("right") {
                collect_assigned_property_types(content, right, env, index, context, updates);
            }
            apply_type_assignment(content, node, env, index, context);

            if let (Some(left), Some(right)) = (
                node.child_by_field_name("left"),
                node.child_by_field_name("right"),
            ) && let Some(property) = this_property_name(content, left)
                && let Some(type_info) = resolve_expr_type(content, right, env, index, context)
            {
                updates.push((property, type_info));
            }
        }
        _ => {
            for child in named_children(node) {
                collect_assigned_property_types(content, child, env, index, context, updates);
            }
        }
    }
}

fn infer_routine_summaries(content: &str, context: &PhpContext, index: &mut PhpSemanticIndex<'_>) {
    let keys = index.routines.keys().cloned().collect::<Vec<_>>();

    for key in keys {
        let Some(routine) = index.routines.get(&key).cloned() else {
            continue;
        };
        let Some(body) = routine.body else {
            continue;
        };

        let param_method_calls = collect_param_method_calls(content, body, &routine.parameters);
        let inferred_return_type = routine.return_type.clone().or_else(|| {
            let mut env = TypeEnv::for_routine(index, &routine);
            infer_return_type(content, body, &mut env, index, context)
        });

        if let Some(routine) = index.routines.get_mut(&key) {
            routine.param_method_calls = param_method_calls;
            routine.return_type = inferred_return_type;
        }
    }
}

fn collect_param_method_calls(
    content: &str,
    body: Node<'_>,
    parameters: &[String],
) -> Vec<ParamMethodCall> {
    let origins = parameters
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), index))
        .collect::<HashMap<_, _>>();
    let mut env = origins;
    let mut calls = Vec::new();

    collect_param_method_calls_in_node(content, body, &mut env, &mut calls);
    calls
}

fn collect_param_method_calls_in_node(
    content: &str,
    node: Node<'_>,
    env: &mut HashMap<String, usize>,
    calls: &mut Vec<ParamMethodCall>,
) {
    match node.kind() {
        "method_declaration" | "function_definition" => {}
        "assignment_expression" => {
            if let Some(right) = node.child_by_field_name("right") {
                collect_param_method_calls_in_node(content, right, env, calls);
            }

            if let Some(left) = node.child_by_field_name("left")
                && let Some(variable) = variable_receiver_name(content, left)
            {
                if let Some(right) = node.child_by_field_name("right")
                    && let Some(param_index) = resolve_param_origin(content, right, env)
                {
                    env.insert(variable, param_index);
                } else {
                    env.remove(&variable);
                }
            }
        }
        "member_call_expression" | "nullsafe_member_call_expression" => {
            if let Some(object) = node.child_by_field_name("object") {
                collect_param_method_calls_in_node(content, object, env, calls);
            }
            if let Some(arguments) = node.child_by_field_name("arguments") {
                collect_param_method_calls_in_node(content, arguments, env, calls);
            }

            if let Some(object) = node.child_by_field_name("object")
                && let Some(param_index) = resolve_param_origin(content, object, env)
                && let Some((method_name, _)) = method_name_from_call(content, node)
            {
                calls.push(ParamMethodCall {
                    param_index,
                    method_name,
                });
            }
        }
        "function_call_expression" | "scoped_call_expression" => {
            if let Some(arguments) = node.child_by_field_name("arguments") {
                collect_param_method_calls_in_node(content, arguments, env, calls);
            }
        }
        _ => {
            for child in named_children(node) {
                collect_param_method_calls_in_node(content, child, env, calls);
            }
        }
    }
}

fn infer_return_type(
    content: &str,
    node: Node<'_>,
    env: &mut TypeEnv,
    index: &PhpSemanticIndex<'_>,
    context: &PhpContext,
) -> Option<TypeInfo> {
    match node.kind() {
        "method_declaration" | "function_definition" => None,
        "assignment_expression" => {
            if let Some(right) = node.child_by_field_name("right")
                && let Some(return_type) = infer_return_type(content, right, env, index, context)
            {
                apply_type_assignment(content, node, env, index, context);
                return Some(return_type);
            }

            apply_type_assignment(content, node, env, index, context);
            None
        }
        "return_statement" => first_named_child(node)
            .and_then(|expression| resolve_expr_type(content, expression, env, index, context))
            .map(|type_info| type_info.with_max_confidence(Confidence::Medium)),
        _ => {
            for child in named_children(node) {
                if let Some(return_type) = infer_return_type(content, child, env, index, context) {
                    return Some(return_type);
                }
            }
            None
        }
    }
}

fn emit_semantic_usages_in_node(
    content: &str,
    node: Node<'_>,
    env: &mut TypeEnv,
    index: &PhpSemanticIndex<'_>,
    context: &PhpContext,
    sink: &mut FactSink<'_>,
) {
    match node.kind() {
        "method_declaration" | "function_definition" => {}
        "assignment_expression" => {
            if let Some(right) = node.child_by_field_name("right") {
                emit_semantic_usages_in_node(content, right, env, index, context, sink);
            }
            apply_type_assignment(content, node, env, index, context);
        }
        "member_call_expression" | "nullsafe_member_call_expression" => {
            if let Some(object) = node.child_by_field_name("object") {
                emit_semantic_usages_in_node(content, object, env, index, context, sink);
            }
            if let Some(arguments) = node.child_by_field_name("arguments") {
                emit_semantic_usages_in_node(content, arguments, env, index, context, sink);
            }

            emit_direct_member_call_usage(content, node, env, index, context, sink);
            emit_forwarded_call_usages(content, node, env, index, context, sink);
        }
        "function_call_expression" | "scoped_call_expression" => {
            if let Some(arguments) = node.child_by_field_name("arguments") {
                emit_semantic_usages_in_node(content, arguments, env, index, context, sink);
            }

            emit_forwarded_call_usages(content, node, env, index, context, sink);
        }
        _ => {
            for child in named_children(node) {
                emit_semantic_usages_in_node(content, child, env, index, context, sink);
            }
        }
    }
}

fn emit_direct_member_call_usage(
    content: &str,
    node: Node<'_>,
    env: &TypeEnv,
    index: &PhpSemanticIndex<'_>,
    context: &PhpContext,
    sink: &mut FactSink<'_>,
) {
    let Some((method_name, method_offset)) = method_name_from_call(content, node) else {
        return;
    };
    let Some(object) = node.child_by_field_name("object") else {
        return;
    };
    let Some(type_info) = resolve_expr_type(content, object, env, index, context) else {
        return;
    };

    sink.usage(
        format!("php:method:{}::{method_name}", type_info.fqcn),
        method_offset,
        type_info.confidence,
        "instance-call",
    );
}

fn emit_forwarded_call_usages(
    content: &str,
    node: Node<'_>,
    env: &TypeEnv,
    index: &PhpSemanticIndex<'_>,
    context: &PhpContext,
    sink: &mut FactSink<'_>,
) {
    let Some(call) = local_call_parts(content, node) else {
        return;
    };
    let Some(summary) = index.routines.get(&routine_key(&call.name)) else {
        return;
    };

    for param_call in &summary.param_method_calls {
        let Some(argument) = call.arguments.get(param_call.param_index) else {
            continue;
        };
        let Some(type_info) = resolve_expr_type(content, *argument, env, index, context) else {
            continue;
        };
        let parameter_name = summary
            .parameters
            .get(param_call.param_index)
            .map(String::as_str);

        if let Some(parameter_type) = parameter_name.and_then(|name| summary.param_types.get(name))
            && parameter_type.fqcn == type_info.fqcn
        {
            continue;
        }

        let type_info = type_info.with_max_confidence(Confidence::Medium);
        sink.usage(
            format!("php:method:{}::{}", type_info.fqcn, param_call.method_name),
            call.name_offset,
            type_info.confidence,
            "forwarded-call",
        );
    }
}

fn apply_type_assignment(
    content: &str,
    node: Node<'_>,
    env: &mut TypeEnv,
    index: &PhpSemanticIndex<'_>,
    context: &PhpContext,
) {
    let Some(left) = node.child_by_field_name("left") else {
        return;
    };
    let Some(right) = node.child_by_field_name("right") else {
        return;
    };
    let right_type = resolve_expr_type(content, right, env, index, context);

    if let Some(variable) = variable_receiver_name(content, left) {
        if let Some(type_info) = right_type {
            env.variables.insert(variable, type_info);
        } else {
            env.variables.remove(&variable);
        }
    } else if let Some(property) = this_property_name(content, left) {
        if let Some(type_info) = right_type {
            env.properties.insert(property, type_info);
        } else {
            env.properties.remove(&property);
        }
    }
}

fn resolve_expr_type(
    content: &str,
    node: Node<'_>,
    env: &TypeEnv,
    index: &PhpSemanticIndex<'_>,
    context: &PhpContext,
) -> Option<TypeInfo> {
    match node.kind() {
        "variable_name" => variable_receiver_name(content, node)
            .and_then(|variable| env.variables.get(&variable).cloned()),
        "member_access_expression" | "nullsafe_member_access_expression" => {
            this_property_name(content, node)
                .and_then(|property| env.properties.get(&property).cloned())
        }
        "object_creation_expression" => object_creation_type(content, node, context),
        "member_call_expression" | "nullsafe_member_call_expression" => {
            container_get_type(content, node, context)
                .or_else(|| local_call_return_type(content, node, index))
        }
        "function_call_expression" | "scoped_call_expression" => {
            local_call_return_type(content, node, index)
        }
        "parenthesized_expression" => first_named_child(node)
            .and_then(|child| resolve_expr_type(content, child, env, index, context)),
        "assignment_expression" => node
            .child_by_field_name("right")
            .and_then(|right| resolve_expr_type(content, right, env, index, context)),
        _ => None,
    }
}

fn resolved_node_type(content: &str, node: Node<'_>, context: &PhpContext) -> Option<TypeInfo> {
    let type_node = node.child_by_field_name("type")?;
    resolved_type_node(content, type_node, context, Confidence::High)
}

fn resolved_type_node(
    content: &str,
    type_node: Node<'_>,
    context: &PhpContext,
    confidence: Confidence,
) -> Option<TypeInfo> {
    for named_type in descendant_kinds(type_node, "named_type") {
        if let Ok(raw_type) = named_type.utf8_text(content.as_bytes())
            && let Some(fqcn) = context.resolve_class_name(raw_type, named_type.start_byte())
            && let Some(type_info) = TypeInfo::new(fqcn, confidence)
        {
            return Some(type_info);
        }
    }

    None
}

fn object_creation_type(content: &str, node: Node<'_>, context: &PhpContext) -> Option<TypeInfo> {
    for child in named_children(node) {
        if !matches!(child.kind(), "name" | "qualified_name" | "relative_name") {
            continue;
        }
        let raw_name = child.utf8_text(content.as_bytes()).ok()?;
        let fqcn = context.resolve_class_name(raw_name, child.start_byte())?;
        return TypeInfo::new(fqcn, Confidence::High);
    }

    None
}

fn container_get_type(content: &str, node: Node<'_>, context: &PhpContext) -> Option<TypeInfo> {
    let (method_name, _) = method_name_from_call(content, node)?;
    if method_name != "get" {
        return None;
    }

    call_arguments(node)
        .first()
        .and_then(|argument| service_lookup_arg_type(content, *argument, context))
}

fn service_lookup_arg_type(
    content: &str,
    node: Node<'_>,
    context: &PhpContext,
) -> Option<TypeInfo> {
    match node.kind() {
        "class_constant_access_expression" => {
            let raw = node.utf8_text(content.as_bytes()).ok()?;
            let (class_name, constant) = raw.rsplit_once("::")?;
            if constant.trim() != "class" {
                return None;
            }
            let fqcn = context.resolve_class_name(class_name.trim(), node.start_byte())?;
            TypeInfo::new(fqcn, Confidence::Medium)
        }
        "string" => {
            let value = php_string_value(node.utf8_text(content.as_bytes()).ok()?)?;
            let fqcn = context.resolve_class_name(&value, node.start_byte())?;
            TypeInfo::new(fqcn, Confidence::Medium)
        }
        _ => None,
    }
}

fn local_call_return_type(
    content: &str,
    node: Node<'_>,
    index: &PhpSemanticIndex<'_>,
) -> Option<TypeInfo> {
    let call = local_call_parts(content, node)?;
    index
        .routines
        .get(&routine_key(&call.name))
        .and_then(|summary| summary.return_type.clone())
}

fn local_call_parts<'tree>(content: &str, node: Node<'tree>) -> Option<CallParts<'tree>> {
    match node.kind() {
        "member_call_expression" | "nullsafe_member_call_expression" => {
            let object = node.child_by_field_name("object")?;
            if !is_this_expr(content, object) {
                return None;
            }
            let (name, name_offset) = method_name_from_call(content, node)?;
            Some(CallParts {
                name,
                name_offset,
                arguments: call_arguments(node),
            })
        }
        "function_call_expression" => {
            let function = node.child_by_field_name("function")?;
            if function.kind() != "name" {
                return None;
            }
            let name = function.utf8_text(content.as_bytes()).ok()?.to_string();
            Some(CallParts {
                name,
                name_offset: function.start_byte(),
                arguments: call_arguments(node),
            })
        }
        "scoped_call_expression" => {
            let scope = node.child_by_field_name("scope")?;
            let scope_text = scope.utf8_text(content.as_bytes()).ok()?;
            if !matches!(scope_text, "self" | "static" | "parent") {
                return None;
            }
            let (name, name_offset) = method_name_from_call(content, node)?;
            Some(CallParts {
                name,
                name_offset,
                arguments: call_arguments(node),
            })
        }
        _ => None,
    }
}

fn method_name_from_call(content: &str, node: Node<'_>) -> Option<(String, usize)> {
    let name_node = node.child_by_field_name("name")?;
    if name_node.kind() != "name" {
        return None;
    }

    Some((
        name_node.utf8_text(content.as_bytes()).ok()?.to_string(),
        name_node.start_byte(),
    ))
}

fn call_arguments<'tree>(node: Node<'tree>) -> Vec<Node<'tree>> {
    let Some(arguments) = node.child_by_field_name("arguments") else {
        return Vec::new();
    };

    named_children(arguments)
        .into_iter()
        .filter_map(argument_value_node)
        .collect()
}

fn argument_value_node(node: Node<'_>) -> Option<Node<'_>> {
    if node.kind() != "argument" {
        return Some(node);
    }

    let name_field = node.child_by_field_name("name");

    named_children(node).into_iter().find(|child| {
        if child.kind() == "variadic_unpacking" {
            return false;
        }
        if let Some(name_field) = name_field
            && child.kind() == name_field.kind()
            && child.start_byte() == name_field.start_byte()
            && child.end_byte() == name_field.end_byte()
        {
            return false;
        }

        true
    })
}

fn resolve_param_origin(
    content: &str,
    node: Node<'_>,
    env: &HashMap<String, usize>,
) -> Option<usize> {
    match node.kind() {
        "variable_name" => {
            variable_receiver_name(content, node).and_then(|variable| env.get(&variable).copied())
        }
        "parenthesized_expression" => {
            first_named_child(node).and_then(|child| resolve_param_origin(content, child, env))
        }
        "assignment_expression" => node
            .child_by_field_name("right")
            .and_then(|right| resolve_param_origin(content, right, env)),
        _ => None,
    }
}

fn first_named_child(node: Node<'_>) -> Option<Node<'_>> {
    named_children(node).into_iter().next()
}

fn named_children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
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

        for child in named_children(node) {
            stack.push(child);
        }
    }

    matches
}

fn property_names(content: &str, root: Node<'_>) -> Vec<String> {
    let mut names = Vec::new();

    for child in named_children(root) {
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

fn this_property_name(content: &str, node: Node<'_>) -> Option<String> {
    let name_node = node.child_by_field_name("name")?;
    if name_node.kind() != "name" {
        return None;
    }

    let object_node = node.child_by_field_name("object")?;
    if !is_this_expr(content, object_node) {
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

fn is_this_expr(content: &str, node: Node<'_>) -> bool {
    if node.kind() == "parenthesized_expression" {
        return first_named_child(node).is_some_and(|child| is_this_expr(content, child));
    }

    node.kind() == "variable_name"
        && node
            .utf8_text(content.as_bytes())
            .is_ok_and(|text| text == "$this")
}

fn php_string_value(raw: &str) -> Option<String> {
    let quote = raw.as_bytes().first().copied()?;
    if !matches!(quote, b'\'' | b'"') || raw.as_bytes().last().copied() != Some(quote) {
        return None;
    }

    Some(raw[1..raw.len().saturating_sub(1)].replace("\\\\", "\\"))
}

fn routine_key(name: &str) -> String {
    name.to_ascii_lowercase()
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
    fn extracts_instance_method_calls_from_aliases_factories_and_new_objects() {
        let facts = usage_facts(
            r#"<?php
namespace Swag\Demo;

use Shopware\Core\Checkout\Document\Service\DocumentGenerator;

final class Demo
{
    public function __construct(private readonly DocumentGenerator $documentGenerator)
    {
    }

    private function declaredGenerator(): DocumentGenerator
    {
        return $this->documentGenerator;
    }

    private function inferredGenerator()
    {
        return $this->documentGenerator;
    }

    public function test(): void
    {
        $alias = $this->documentGenerator;
        $declared = $this->declaredGenerator();
        $inferred = $this->inferredGenerator();
        $created = new DocumentGenerator();

        $alias->generate([], []);
        $declared->generate([], []);
        $inferred->generate([], []);
        $created->generate([], []);
        (new DocumentGenerator())->generate([], []);
        $this->declaredGenerator()->generate([], []);
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

        assert_eq!(matches, 6);
    }

    #[test]
    fn extracts_forwarded_instance_method_calls_from_same_file_wrappers() {
        let facts = usage_facts(
            r#"<?php
namespace Swag\Demo;

use Shopware\Core\Checkout\Document\Service\DocumentGenerator;

final class Demo
{
    public function __construct(private readonly DocumentGenerator $documentGenerator)
    {
    }

    private function invokeGenerator($generator): void
    {
        $alias = $generator;
        $alias->generate([], []);
    }

    public function test(): void
    {
        $this->invokeGenerator($this->documentGenerator);
    }
}
"#,
        );

        assert!(facts.iter().any(|fact| {
            fact.surface.as_str()
                == "php:method:Shopware\\Core\\Checkout\\Document\\Service\\DocumentGenerator::generate"
                && fact.usage_kind == "forwarded-call"
                && fact.confidence == Confidence::Medium
        }));
    }

    #[test]
    fn extracts_instance_method_calls_from_class_service_lookups() {
        let facts = usage_facts(
            r#"<?php
namespace Swag\Demo;

use Shopware\Core\Checkout\Document\Service\DocumentGenerator;

final class Demo
{
    public function test(): void
    {
        $this->container->get(DocumentGenerator::class)->generate([], []);
        $this->container->get('Shopware\\Core\\Checkout\\Document\\Service\\DocumentGenerator')->generate([], []);
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
                    && fact.confidence == Confidence::Medium
            })
            .count();

        assert_eq!(matches, 2);
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
