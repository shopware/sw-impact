use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use ariadne::{Color, ColorGenerator, Fmt, Label, Report, ReportKind, Source};
use sw_impact_scanner::report::Report as DataReport;

pub fn output_human(report: &DataReport) {
    let mut source_cache = SourceFileCache::default();
    let mut colors = ColorGenerator::new();

    for surface in &report.removed {
        let file = &surface.file_path;
        let source = source_cache.get(file);

        Report::build(
            ReportKind::Error,
            (
                file.as_os_str().to_string_lossy().as_ref(),
                surface.source_range.start_byte..surface.source_range.end_byte,
            ),
        )
        .with_code(1)
        .with_message("Removed public API surface")
        .with_label(
            Label::new((
                file.as_os_str().to_string_lossy().as_ref(),
                surface.source_range.start_byte..surface.source_range.end_byte,
            ))
            .with_message("this was removed"),
        )
        .finish()
        .print((
            file.as_path().to_string_lossy().as_ref(),
            Source::from(source),
        ))
        .unwrap();
    }
}

#[derive(Debug, Default)]
struct SourceFileCache {
    files: HashMap<PathBuf, String>,
}

impl SourceFileCache {
    // TODO: error handling?
    fn get(&mut self, path: impl AsRef<Path>) -> &str {
        let path = path.as_ref();

        if !self.files.contains_key(path) {
            let content = std::fs::read_to_string(path).unwrap();
            self.files.insert(path.to_path_buf(), content);
        }

        self.files.get(path).unwrap()
    }
}
