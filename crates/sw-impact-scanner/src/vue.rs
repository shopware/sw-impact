use std::{path::Path, sync::LazyLock};

use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator};

static QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        include_str!("queries/vue.scm"),
    )
    .unwrap()
});

pub fn process_file_vue(path: &Path) {
    println!("Admin vue detected");

    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();

    let file_content = std::fs::read_to_string(path).unwrap();

    let tree = parser.parse(file_content.as_bytes(), None).unwrap();
    let root = tree.root_node();

    let mut cursor = QueryCursor::new();

    let matches = cursor.matches(&QUERY, root, file_content.as_bytes());
    let capture_names = QUERY.capture_names();
    matches.for_each(|m| {
        for c in m.captures {
            let name = &capture_names[c.index as usize];
            if name.starts_with('_') {
                continue; // ignore captures starting with underscore
            }

            println!("{}", c.node.utf8_text(file_content.as_bytes()).unwrap());
        }
    });
}
