use std::{path::Path, sync::LazyLock};

use tracing::info;
use tree_sitter::{Language, Parser, Query, QueryCursor, StreamingIterator};

use crate::api::{Surface, SurfaceCollector, VueMethod};

static QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        include_str!("queries/vue.scm"),
    )
    .unwrap_or_else(|err| panic!("failed to compile tree-sitter query queries/vue.scm: {err}"))
});

pub fn validate_queries() {
    let _ = &*QUERY;
}

pub fn process_file_vue(path: &Path, collector: &SurfaceCollector) {
    let lang: &Language = &tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    let kind_id_method = lang.id_for_node_kind("method_definition", true);
    let field_id_name: u16 = lang.field_id_for_name("name").unwrap().into();
    let field_id_parameters: u16 = lang.field_id_for_name("parameters").unwrap().into();
    let field_id_return_type: u16 = lang.field_id_for_name("return_type").unwrap().into();

    let mut parser = Parser::new();
    parser.set_language(lang).unwrap();

    let component_name = extract_component_name(path).unwrap();
    info!("parsing {}", &component_name);
    let file_content = std::fs::read_to_string(path).unwrap();

    let tree = parser.parse(file_content.as_bytes(), None).unwrap();
    let root = tree.root_node();

    let mut cursor = QueryCursor::new();
    let matches = cursor.matches(&QUERY, root, file_content.as_bytes());
    let capture_names = QUERY.capture_names();
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

fn extract_component_name(path: &Path) -> Option<String> {
    // TODO: make this more reliable
    Some(path.parent()?.file_name()?.to_str()?.to_owned())
}
