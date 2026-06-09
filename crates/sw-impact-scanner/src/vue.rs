use std::{path::Path, sync::LazyLock};

use tracing::info;
use tree_sitter::{Language, Node, Parser, Query, QueryCursor, StreamingIterator};

use crate::api::{Surface, SurfaceCollector, VueMethod, VueProp};

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

    let component_name = extract_component_name(path).unwrap();
    info!("parsing {}", &component_name);
    let file_content = std::fs::read_to_string(path).unwrap();

    let tree = parser.parse(file_content.as_bytes(), None).unwrap();
    let root = tree.root_node();

    let vue_file = scan_file_top_level(root, file_content.as_bytes());
    let Some(obj) = vue_file.options_obj else {
        info!("skipping (no vue component): {}", path.display());
        return;
    };
    if vue_file.is_private {
        info!("skipping (is private): {}", path.display());
        return;
    }

    let mut cursor = QueryCursor::new();
    let matches = cursor.matches(&QUERY_OBJ, obj, file_content.as_bytes());
    let capture_names = QUERY_OBJ.capture_names();
    matches.for_each(|m| {
        for c in m.captures {
            let capture_name = &capture_names[c.index as usize];
            if capture_name.starts_with('_') {
                continue; // ignore captures starting with underscore
            }

            if c.node.kind_id() == kind_id_method {
                let method_name = c.node.child_by_field_id(field_id_name).unwrap();
                let params = c.node.child_by_field_id(field_id_parameters).unwrap();
                let return_type = c.node.child_by_field_id(field_id_return_type);

                collector.push(Surface::with_signature(
                    path.to_owned(),
                    format!(
                        "vue.{}.{}.{}",
                        component_name,
                        capture_name,
                        method_name.utf8_text(file_content.as_bytes()).unwrap()
                    ),
                    crate::api::Signature::VueMethod(VueMethod {
                        parameters: params
                            .utf8_text(file_content.as_bytes())
                            .unwrap()
                            .to_owned(),
                        return_type: return_type
                            .map(|n| n.utf8_text(file_content.as_bytes()).unwrap().to_owned()),
                    }),
                ));

                continue;
            }

            if c.node.kind_id() == kind_id_pair && *capture_name == "prop" {
                let key = c.node.child_by_field_id(field_id_key).unwrap();
                let value = c.node.child_by_field_id(field_id_value).unwrap();

                collector.push(Surface::with_signature(
                    path.to_owned(),
                    format!(
                        "vue.{}.{}.{}",
                        component_name,
                        capture_name,
                        key.utf8_text(file_content.as_bytes()).unwrap()
                    ),
                    crate::api::Signature::VueProp(VueProp {
                        definition: value.utf8_text(file_content.as_bytes()).unwrap().to_owned(),
                    }),
                ));

                continue;
            }

            collector.push(Surface::new(
                path.to_owned(),
                format!(
                    "vue.{}.{}.{}",
                    component_name,
                    capture_name,
                    c.node.utf8_text(file_content.as_bytes()).unwrap()
                ),
            ));
        }
    });
}

struct VueFile<'a> {
    is_private: bool,
    options_obj: Option<Node<'a>>,
}

/// Returns the TS / JS object defining the vue component with options api
/// If it exists or nothing if the component is declared as @private in a comment
fn scan_file_top_level<'a>(root: Node<'a>, source: &[u8]) -> VueFile<'a> {
    let mut cursor = QueryCursor::new();
    let matches = cursor.matches(&QUERY_FILE, root, source);
    let capture_names = QUERY_FILE.capture_names();

    let mut options_obj = None;
    let mut is_private = false;
    matches.for_each(|m| {
        for c in m.captures {
            let capture_name = &capture_names[c.index as usize];

            if *capture_name == "component" {
                options_obj = Some(c.node);
                continue;
            }

            if *capture_name == "toplevel.comment" {
                let text = c.node.utf8_text(source).unwrap();

                if text.contains("@private") {
                    is_private = true;
                }
            }
        }
    });

    VueFile {
        is_private,
        options_obj,
    }
}

fn extract_component_name(path: &Path) -> Option<String> {
    // TODO: make this more reliable
    Some(path.parent()?.file_name()?.to_str()?.to_owned())
}
