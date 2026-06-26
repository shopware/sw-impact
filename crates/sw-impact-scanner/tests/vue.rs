use std::path::PathBuf;

use insta::Settings;
use sw_impact_scanner::api::SurfaceCollector;

fn run_fixture_test(fixture: impl Into<PathBuf>) {
    let path: PathBuf = fixture.into();
    let name = path
        .file_name()
        .expect("fixture filename")
        .to_string_lossy();
    let src = std::fs::read_to_string(&path)
        .expect(&format!("failed loading fixture {}", path.display()));

    let collector = SurfaceCollector::new();
    sw_impact_scanner::vue::process_vue(&path, &path, src.as_bytes(), &collector);
    let surfaces = collector.finish();

    let mut settings = Settings::clone_current();
    settings.set_omit_expression(true);
    settings.set_sort_maps(true);
    settings.set_input_file(&path);
    settings.set_description(src);

    settings.bind(|| {
        insta::assert_yaml_snapshot!(name.as_ref(), surfaces);
    });
}

#[test]
fn test_vue_basic_js() {
    run_fixture_test("tests/fixtures/vue-basic.js");
}

#[test]
fn test_vue_props_array() {
    run_fixture_test("tests/fixtures/vue-props-array.js");
}

#[test]
fn test_vue_props_object() {
    run_fixture_test("tests/fixtures/vue-props-object.js");
}
