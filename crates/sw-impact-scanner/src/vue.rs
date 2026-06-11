use std::{path::Path, sync::LazyLock};

use tracing::{error, info, warn};
use tree_sitter::{Language, Node, Parser, Query, QueryCursor, StreamingIterator};

use crate::api::{Surface, SurfaceCollector, TsMethod, TsParam, VueProp};

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
    let _ = &*QUERY_FILE;
    let _ = &*QUERY_OBJ;
}

pub fn process_file_vue(path: &Path, collector: &SurfaceCollector) {
    let lang: &Language = &tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    let kind_id_method = lang.id_for_node_kind("method_definition", true);
    let kind_id_pair = lang.id_for_node_kind("pair", true);
    let field_id_name: u16 = lang.field_id_for_name("name").unwrap().into();
    let field_id_parameters: u16 = lang.field_id_for_name("parameters").unwrap().into();
    let field_id_return_type: u16 = lang.field_id_for_name("return_type").unwrap().into();
    let field_id_key: u16 = lang.field_id_for_name("key").unwrap().into();
    let field_id_value: u16 = lang.field_id_for_name("value").unwrap().into();

    let mut parser = Parser::new();
    parser.set_language(lang).unwrap();

    let file_content = std::fs::read_to_string(path).unwrap();

    let tree = parser.parse(file_content.as_bytes(), None).unwrap();
    let root = tree.root_node();

    let vue_file = scan_file_top_level(root, file_content.as_bytes(), path);
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
    let matches = cursor.matches(&QUERY_OBJ, obj, file_content.as_bytes());
    let capture_names = QUERY_OBJ.capture_names();
    let mut surface_count = 0;
    matches.for_each(|m| {
        for c in m.captures {
            let capture_name = &capture_names[c.index as usize];
            if capture_name.starts_with('_') {
                continue; // ignore captures starting with underscore
            }

            if c.node.kind_id() == kind_id_method {
                let is_async = c.node.child(0).is_some_and(|child| child.kind() == "async");
                let method_name = c.node.child_by_field_id(field_id_name).unwrap();
                let params = c.node.child_by_field_id(field_id_parameters).unwrap();
                let return_type = c
                    .node
                    .child_by_field_id(field_id_return_type)
                    .and_then(|n| n.named_child(0)); // skip ':' and access inner node

                let fqn = format!(
                    "vue.{}.{}.{}",
                    component_name,
                    capture_name,
                    method_name.utf8_text(file_content.as_bytes()).unwrap()
                );

                collector.push(
                    Surface::builder()
                        .file_path(path.to_owned())
                        .source_range(method_name.range())
                        .signature(TsMethod {
                            is_async,
                            parameters: extract_ts_method_params(
                                &fqn,
                                params,
                                file_content.as_bytes(),
                            ),
                            return_type: return_type.map(|n| {
                                normalize_ws(n.utf8_text(file_content.as_bytes()).unwrap())
                            }),
                        })
                        .fqn(fqn)
                        .build()
                        .unwrap(),
                );
                surface_count += 1;

                continue;
            }

            if c.node.kind_id() == kind_id_pair && *capture_name == "prop" {
                let key = c.node.child_by_field_id(field_id_key).unwrap();
                let value = c.node.child_by_field_id(field_id_value).unwrap();

                collector.push(
                    Surface::builder()
                        .file_path(path.to_owned())
                        .source_range(key.range())
                        .fqn(format!(
                            "vue.{}.{}.{}",
                            component_name,
                            capture_name,
                            key.utf8_text(file_content.as_bytes()).unwrap()
                        ))
                        .signature(VueProp {
                            definition: normalize_ws(
                                value.utf8_text(file_content.as_bytes()).unwrap(),
                            ),
                        })
                        .build()
                        .unwrap(),
                );
                surface_count += 1;

                continue;
            }

            collector.push(
                Surface::builder()
                    .file_path(path.to_owned())
                    .source_range(c.node.range())
                    .fqn(format!(
                        "vue.{}.{}.{}",
                        component_name,
                        capture_name,
                        c.node.utf8_text(file_content.as_bytes()).unwrap()
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

struct VueFile<'a> {
    is_private: bool,
    component_name: Option<String>,
    options_obj: Option<Node<'a>>,
}

/// Returns the TS / JS object defining the vue component with options api
/// If it exists or nothing if the component is declared as @private in a comment
fn scan_file_top_level<'a>(root: Node<'a>, source: &[u8], path: &Path) -> VueFile<'a> {
    let mut cursor = QueryCursor::new();
    let matches = cursor.matches(&QUERY_FILE, root, source);
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
                let text = c.node.utf8_text(source).unwrap();

                if text.contains("@private") {
                    vue_file.is_private = true;
                }
                continue;
            }

            if *capture_name == "import" {
                let text = c.node.utf8_text(source).unwrap();
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

fn extract_ts_method_params(fqn: &str, n: Node, source: &[u8]) -> Vec<TsParam> {
    let mut cursor = n.walk();

    n.named_children(&mut cursor)
        .filter_map(|param| ts_param_from_tree_sitter(fqn, param, source))
        .collect()
}

fn ts_param_from_tree_sitter(fqn: &str, param: Node, source: &[u8]) -> Option<TsParam> {
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
                name: normalize_ws(name.utf8_text(source).unwrap()),
                type_annotation: param
                    .child_by_field_name("type")
                    .and_then(|n| n.named_child(0)) // access inner node of 'type_annotation' to skip ':'
                    .map(|n| normalize_ws(n.utf8_text(source).unwrap())),
                default_value: param
                    .child_by_field_name("value")
                    .map(|n| normalize_ws(n.utf8_text(source).unwrap())),
                is_optional: param.kind() == "optional_parameter",
                is_rest: pattern.kind() == "rest_pattern",
            })
        }

        _ => {
            error!(
                "failed to parse TS method param in {}, ignoring: {}",
                fqn,
                param.utf8_text(source).unwrap()
            );
            None
        }
    }
}

/// Replaces all whitespace with a single space
fn normalize_ws(s: &str) -> String {
    // TODO: check performance impact on this, could be optimized if needed
    s.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
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
