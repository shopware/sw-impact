use std::{path::Path, sync::LazyLock};
use tracing::{error, info, warn};
use tree_sitter::{Language, Node, Parser, Query, QueryCursor, Range, StreamingIterator};

use crate::api::{SourceToken, Surface, SurfaceCollector, TsMethod, TsParam, VueProp};

static QUERY_FILE: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        include_str!("queries/vue-shopware-file.scm"),
    )
    .unwrap_or_else(|err| {
        panic!("failed to compile tree-sitter query queries/vue-shopware-file.scm: {err}")
    })
});
static QUERY_OBJ: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        include_str!("queries/vue-options-api.scm"),
    )
    .unwrap_or_else(|err| {
        panic!("failed to compile tree-sitter query queries/vue-options-api.scm: {err}")
    })
});

pub fn validate_queries() {
    // initialize LazyLocks and panic on failure
    let _ = &*QUERY_FILE;
    let _ = &*QUERY_OBJ;
}

pub fn process_file_vue(path: &Path, repo_path: &Path, collector: &SurfaceCollector) {
    let file_content = std::fs::read_to_string(path).unwrap();
    let src = file_content.as_bytes();

    process_vue(path, repo_path, src, collector);
}

pub fn process_vue(path: &Path, repo_path: &Path, src: &[u8], collector: &SurfaceCollector) {
    let lang: &Language = &tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();

    let mut parser = Parser::new();
    parser.set_language(lang).unwrap();

    let tree = parser.parse(src, None).unwrap();
    let root = tree.root_node();

    let vue_file = scan_file_top_level(root, src, repo_path);
    let Some(obj) = vue_file.options_obj else {
        info!("skipping (no vue component): {}", path.display());
        return;
    };
    if vue_file.is_private {
        info!("skipping (is private): {}", path.display());
        return;
    }
    let Some(component_name) = vue_file.component_name else {
        warn!(
            "skipping (failed component name extraction): {}",
            path.display()
        );
        return;
    };

    info!("parsing {} from ({})", &component_name, path.display());
    let mut cursor = QueryCursor::new();
    let matches = cursor.matches(&QUERY_OBJ, obj, src);
    let capture_names = QUERY_OBJ.capture_names();
    let mut surface_count = 0;
    matches.for_each(|m| {
        for c in m.captures {
            let capture_name = &capture_names[c.index as usize];
            if capture_name.starts_with('_') {
                continue; // ignore captures starting with underscore
            }

            if c.node.kind() == "method_definition" {
                let r#async = c.node.child(0).filter(|child| child.kind() == "async");
                let method_name = c.node.child_by_field_name("name").unwrap();
                let params = c.node.child_by_field_name("parameters").unwrap();
                let return_type = c
                    .node
                    .child_by_field_name("return_type")
                    .and_then(|n| n.named_child(0)); // skip ':' and access inner node

                let fqn = format!(
                    "vue.{}.{}.{}",
                    component_name,
                    capture_name,
                    method_name.utf8_text(src).unwrap()
                );

                collector.push(
                    Surface::builder()
                        .file_path(repo_path.to_owned())
                        .source_token(SourceToken::from_range_source(
                            method_signature_range(c.node),
                            src,
                        ))
                        .signature(TsMethod {
                            r#async: r#async.map(|n| SourceToken::from_node_source(n, src)),
                            parameters: extract_ts_method_params(&fqn, params, src),
                            return_type: return_type.map(|n| SourceToken::from_node_source(n, src)),
                        })
                        .fqn(fqn)
                        .build()
                        .unwrap(),
                );
                surface_count += 1;

                continue;
            }

            if *capture_name == "props.value" {
                match c.node.kind() {
                    "array" => {
                        let mut cursor = c.node.walk();

                        for child in c.node.named_children(&mut cursor) {
                            if child.kind() == "string"
                                && let Some(prop_name) = string_value(child, src)
                            {
                                collector.push(
                                    Surface::builder()
                                        .file_path(repo_path.to_owned())
                                        .source_token(SourceToken::from_node_source(child, src))
                                        .fqn(format!("vue.{}.prop.{}", component_name, &prop_name))
                                        .signature(VueProp {
                                            type_annotation: None,
                                            required: None,
                                        })
                                        .build()
                                        .unwrap(),
                                );
                                surface_count += 1;
                            }
                        }
                    }
                    "object" => {
                        let mut cursor = c.node.walk();

                        for child in c.node.named_children(&mut cursor) {
                            if child.kind() != "pair" {
                                continue;
                            }

                            let Some(key_node) = child.child_by_field_name("key") else {
                                continue;
                            };

                            let Some(value_node) = child.child_by_field_name("value") else {
                                continue;
                            };

                            if let Some(prop_def) =
                                parse_prop_definition(value_node, src, repo_path)
                            {
                                let prop_name = key_node.utf8_text(src).unwrap();
                                collector.push(
                                    Surface::builder()
                                        .file_path(repo_path.to_owned())
                                        .source_token(SourceToken::from_node_source(key_node, src))
                                        .fqn(format!("vue.{}.prop.{}", component_name, prop_name))
                                        .signature(prop_def)
                                        .build()
                                        .unwrap(),
                                );
                                surface_count += 1;
                            }
                        }
                    }
                    k => {
                        error!(
                            "failed to parse vue prop declaration of kind {} in {} with text: {}",
                            k,
                            path.display(),
                            c.node.utf8_text(src).unwrap()
                        );
                    }
                }

                continue;
            }

            collector.push(
                Surface::builder()
                    .file_path(repo_path.to_owned())
                    .source_token(SourceToken::from_node_source(c.node, src))
                    .fqn(format!(
                        "vue.{}.{}.{}",
                        component_name,
                        capture_name,
                        c.node.utf8_text(src).unwrap()
                    ))
                    .build()
                    .unwrap(),
            );
            surface_count += 1;
        }
    });

    if surface_count > 0 && !component_name.starts_with("sw-") {
        error!(
            "found surfaces in file {} with detected component name {} that doesn't start with 'sw-'",
            path.display(),
            component_name
        );
    }
}

fn parse_prop_definition(value_node: Node, src: &[u8], path: &Path) -> Option<VueProp> {
    match value_node.kind() {
        // title: String
        // input: [Number, String]
        "identifier" | "array" => Some(VueProp {
            type_annotation: Some(SourceToken::from_node_source(value_node, src)),
            required: None,
        }),
        // count: { type: Number, required: true }
        // msg: { type: [String, Number] }
        "object" => Some(parse_prop_object(value_node, src)),
        k => {
            error!(
                "failed to parse vue prop declaration of kind {} in {} with text: {}",
                k,
                path.display(),
                value_node.utf8_text(src).unwrap()
            );
            None
        }
    }
}

fn parse_prop_object(obj_node: Node, src: &[u8]) -> VueProp {
    let mut prop = VueProp {
        type_annotation: None,
        required: None,
    };

    let mut cursor = obj_node.walk();
    for child in obj_node.named_children(&mut cursor) {
        if child.kind() != "pair" {
            continue;
        }

        let Some(key_node) = child.child_by_field_name("key") else {
            continue;
        };

        let Some(value_node) = child.child_by_field_name("value") else {
            continue;
        };

        let key_str = key_node.utf8_text(src).unwrap();
        match key_str {
            "type" => prop.type_annotation = Some(SourceToken::from_node_source(value_node, src)),
            "required" => {
                let value_str = value_node.utf8_text(src).unwrap();
                if value_str == "true" {
                    prop.required = Some(SourceToken::from_node_source(value_node, src));
                }
            }
            _ => {
                // Other object attributes don't matter for extraction
            }
        };
    }

    prop
}

struct VueFile<'a> {
    is_private: bool,
    component_name: Option<String>,
    options_obj: Option<Node<'a>>,
}

/// Returns the TS / JS object defining the vue component with options api
/// If it exists or nothing if the component is declared as @private / @experimental / @internal in a comment
fn scan_file_top_level<'a>(root: Node<'a>, src: &[u8], path: &Path) -> VueFile<'a> {
    let mut cursor = QueryCursor::new();
    let matches = cursor.matches(&QUERY_FILE, root, src);
    let capture_names = QUERY_FILE.capture_names();

    let mut vue_file = VueFile {
        is_private: false,
        component_name: None,
        options_obj: None,
    };
    matches.for_each(|m| {
        for c in m.captures {
            let capture_name = &capture_names[c.index as usize];

            if *capture_name == "component" {
                vue_file.options_obj = Some(c.node);
                continue;
            }

            if *capture_name == "toplevel.comment" {
                let text = c.node.utf8_text(src).unwrap();

                if text.contains("@private")
                    || text.contains("@experimental")
                    || text.contains("@internal")
                {
                    vue_file.is_private = true;
                }
                continue;
            }

            if *capture_name == "import" {
                let text = c.node.utf8_text(src).unwrap();
                if let Some(component_name) = extract_component_name_from_template(text) {
                    vue_file.component_name = Some(component_name);
                }

                continue;
            }
        }
    });

    if vue_file.component_name.is_none() {
        // component name extraction from template import failed,
        // fallback to path based extraction as last resort
        vue_file.component_name = extract_component_name_from_path(path);
    }

    vue_file
}

fn method_signature_range(method: Node) -> Range {
    let method_range = method.range();
    let mut end_byte = method_range.end_byte;
    let mut end_point = method_range.end_point;
    if let Some(body) = method.child_by_field_name("body") {
        let body_range = body.range();
        end_byte = body_range.start_byte;
        end_point = body_range.start_point;
    }

    Range {
        start_byte: method_range.start_byte,
        end_byte,
        start_point: method_range.start_point,
        end_point,
    }
}

fn extract_ts_method_params(fqn: &str, n: Node, src: &[u8]) -> Vec<TsParam> {
    let mut cursor = n.walk();

    n.named_children(&mut cursor)
        .filter_map(|param| ts_param_from_tree_sitter(fqn, param, src))
        .collect()
}

fn ts_param_from_tree_sitter(fqn: &str, param: Node, src: &[u8]) -> Option<TsParam> {
    match param.kind() {
        // TS / JS:
        // foo
        // foo: string
        // foo = 1
        // foo: string = "x"
        // foo?: string
        // ...rest
        // { destructure }
        "required_parameter" | "optional_parameter" => {
            let pattern = param.child_by_field_name("pattern")?;
            let name = if pattern.kind() == "rest_pattern" {
                // use parameter name without leading ...
                pattern.named_child(0).unwrap()
            } else {
                pattern
            };

            Some(TsParam {
                name: SourceToken::from_node_source(name, src),
                type_annotation: param
                    .child_by_field_name("type")
                    .and_then(|n| n.named_child(0)) // access inner node of 'type_annotation' to skip ':'
                    .map(|n| SourceToken::from_node_source(n, src)),
                default_value: param
                    .child_by_field_name("value")
                    .map(|n| SourceToken::from_node_source(n, src)),
                optional: if param.kind() == "optional_parameter" {
                    let mut cursor = param.walk();
                    param
                        .children(&mut cursor)
                        .find(|c| c.kind() == "?")
                        .map(|n| SourceToken::from_node_source(n, src))
                } else {
                    None
                },
                rest: if pattern.kind() == "rest_pattern" {
                    pattern
                        .child(0)
                        .map(|n| SourceToken::from_node_source(n, src))
                } else {
                    None
                },
            })
        }

        _ => {
            error!(
                "failed to parse TS method param in {}, ignoring: {}",
                fqn,
                param.utf8_text(src).unwrap()
            );
            None
        }
    }
}

fn string_value(node: Node, src: &[u8]) -> Option<String> {
    // tree could look like this
    // string
    //   string_fragment
    //   escape_sequence
    //   string_fragment

    let mut cursor = node.walk();
    let mut value = String::with_capacity(node.end_byte() - node.start_byte());

    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "string_fragment" => {
                value.push_str(child.utf8_text(src).ok()?);
            }

            // keep escape codes as raw text for now
            "escape_sequence" => {
                value.push_str(child.utf8_text(src).ok()?);
            }

            _ => {}
        }
    }

    Some(value)
}

fn extract_component_name_from_template(s: &str) -> Option<String> {
    if s.ends_with(".html.twig") {
        let (_, filename) = s.split_once("/")?;
        if filename.contains('/') {
            // path is more complicated than just ./my-component.html.twig
            return None;
        }

        let (component_name, _) = filename.split_once(".")?;

        return Some(component_name.to_owned());
    }

    None
}

fn extract_component_name_from_path(path: &Path) -> Option<String> {
    Some(path.parent()?.file_name()?.to_str()?.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_extract_component_name_from_template() {
        assert_eq!(
            Some("sw-users-permissions-user-listing".to_owned()),
            extract_component_name_from_template("./sw-users-permissions-user-listing.html.twig")
        );
        assert_eq!(
            None,
            extract_component_name_from_template(
                "./../sw-condition-generic/sw-condition-generic.html.twig"
            )
        );
    }

    #[test]
    fn test_extract_component_name_from_path() {
        assert_eq!(
            Some("sw-users-permissions-user-listing".to_owned()),
            extract_component_name_from_path(&PathBuf::from(
                "components/sw-users-permissions-user-listing/index.js"
            ))
        );
    }
}
