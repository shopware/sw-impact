use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use ariadne::{Label, Report, ReportKind, Source};
use sw_impact_scanner::report::Report as DataReport;

pub fn output_human(report: &DataReport) {
    let mut source_cache = SourceFileCache::default();

    for surface in &report.removed {
        let file = surface.file_path.as_path();
        let file_id = file.display().to_string();
        let source = source_cache.get(file);

        Report::build(
            ReportKind::Error,
            (
                &file_id,
                surface.source_range.start_byte..surface.source_range.end_byte,
            ),
        )
        .with_code("api:removed")
        .with_message("Removed public API surface")
        .with_note(format!("API surface: {}", surface.fqn))
        .with_label(
            // TODO: this label is kind of broken, pointing to the new file source but on something that
            // doesn't exist anymore
            Label::new((
                &file_id,
                surface.source_range.start_byte..surface.source_range.start_byte,
            ))
            .with_message("Previously was defined on this line"),
        )
        .finish()
        .print((&file_id, Source::from(source)))
        .unwrap();
    }

    for signature_change in &report.breaking_changes {
        let file = signature_change.surface.file_path.as_path();
        let file_id = file.display().to_string();
        let source = source_cache.get(file);

        let mut report = Report::build(
            ReportKind::Error,
            (
                &file_id,
                signature_change.surface.source_range.start_byte
                    ..signature_change.surface.source_range.end_byte,
            ),
        )
        .with_code(format!("api:break:{}", signature_change.kind))
        .with_message("Potential breaking change on public API surface")
        .with_note(format!("API surface: {}", signature_change.surface.fqn));

        if let Some(t) = &signature_change.old {
            report = report.with_note(format!("old: {}", &t.text_normalized));
        }
        if let Some(t) = &signature_change.new {
            report = report.with_label(
                Label::new((&file_id, t.source_range.start_byte..t.source_range.end_byte))
                    .with_message("new implementation TODO: details"),
            );

            report = report.with_note(format!("new: {}", &t.text_normalized));
        } else {
            // always add a label
            report = report.with_label(Label::new((&file_id, 0..0)).with_message("In this place"));
        }

        /*
                let label = match signature_change.kind {
                    sw_impact_scanner::report::SignatureChangeKind::TsMethodParamRemoved => todo!(),
                    sw_impact_scanner::report::SignatureChangeKind::TsMethodParamTypeChanged => todo!(),
                    sw_impact_scanner::report::SignatureChangeKind::TsMethodParamRestChanged => todo!(),
                    sw_impact_scanner::report::SignatureChangeKind::TsMethodParamBecameRequired => todo!(),
                    sw_impact_scanner::report::SignatureChangeKind::TsMethodRequiredParamAdded => todo!(),
                    sw_impact_scanner::report::SignatureChangeKind::TsMethodReturnTypeChanged => todo!(),
                    sw_impact_scanner::report::SignatureChangeKind::TsMethodAsyncChanged => todo!(),
                };
        */

        report
            .finish()
            .print((&file_id, Source::from(source)))
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
