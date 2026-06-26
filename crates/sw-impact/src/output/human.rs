use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use ariadne::{Label, Report, ReportKind, Source};
use sw_impact_scanner::report::{Report as DataReport, SignatureChangeKind};

pub fn output_human(report: &DataReport, source_root: &Path) {
    let mut source_cache = SourceFileCache::default();

    for surface in &report.removed {
        let file = surface.file_path.as_path();
        let source_file = source_root.join(file);
        let file_id = file.display().to_string();
        let source = source_cache.get(source_file);

        Report::build(
            ReportKind::Error,
            (
                &file_id,
                surface.source_token.source_range.start_byte
                    ..surface.source_token.source_range.end_byte,
            ),
        )
        .with_code("api:removed")
        .with_message("Removed public API surface")
        .with_help(format!(
            "old signature was: {}",
            surface.source_token.text_normalized
        ))
        .with_note(format!("API surface: {}", surface.fqn))
        .with_label(
            Label::new((
                &file_id,
                surface.source_token.source_range.start_byte
                    ..surface.source_token.source_range.start_byte,
            ))
            .with_message("was previously defined in this line"),
        )
        .finish()
        .print((&file_id, Source::from(source)))
        .unwrap();
    }

    for signature_change in &report.breaking_changes {
        let file = signature_change.base_surface.file_path.as_path();
        let source_file = source_root.join(file);
        let file_id = file.display().to_string();
        let source = source_cache.get(source_file);

        let mut report = Report::build(
            ReportKind::Error,
            (
                &file_id,
                signature_change
                    .base_surface
                    .source_token
                    .source_range
                    .start_byte
                    ..signature_change
                        .base_surface
                        .source_token
                        .source_range
                        .end_byte,
            ),
        )
        .with_code(signature_change.kind)
        .with_message("Potential breaking change on public API surface")
        .with_note(format!(
            "API surface: {}",
            signature_change.base_surface.fqn
        ));

        if !matches!(
            signature_change.kind,
            SignatureChangeKind::VuePropBecameRequired | SignatureChangeKind::VuePropTypeChanged
        ) {
            report = report.with_help(format!(
                "old signature was: {}",
                signature_change.base_surface.source_token.text_normalized
            ));
        }

        let base_surface_text = &signature_change.base_surface.source_token.text_normalized;
        if let Some(t) = &signature_change.old {
            report = report.with_note(format!("before: {}", &t.text_normalized));
        }
        let old_text = signature_change
            .old
            .as_ref()
            .map(|t| t.text_normalized.as_str())
            .unwrap_or("");

        let label = match signature_change.kind {
            sw_impact_scanner::report::SignatureChangeKind::TsMethodParamRemoved => {
                "parameter removed"
            }
            sw_impact_scanner::report::SignatureChangeKind::TsMethodParamTypeChanged => &format!(
                "parameter type changed from '{}' to '{}'",
                old_text, &signature_change.new.text_normalized
            ),
            sw_impact_scanner::report::SignatureChangeKind::TsMethodParamRestChanged => {
                "parameter ...rest changed"
            }
            sw_impact_scanner::report::SignatureChangeKind::TsMethodParamBecameRequired => {
                "parameter became required"
            }
            sw_impact_scanner::report::SignatureChangeKind::TsMethodRequiredParamAdded => {
                "required parameter added"
            }
            sw_impact_scanner::report::SignatureChangeKind::TsMethodReturnTypeChanged => &format!(
                "return type changed from '{}' to '{}'",
                old_text, &signature_change.new.text_normalized
            ),
            sw_impact_scanner::report::SignatureChangeKind::TsMethodAsyncChanged => "async changed",
            SignatureChangeKind::VuePropTypeChanged => &format!(
                "prop '{base_surface_text}' type changed from '{old_text}' to '{}'",
                signature_change.new.text_normalized
            ),
            SignatureChangeKind::VuePropBecameRequired => {
                &format!("prop '{base_surface_text}' became required")
            }
        };
        report = report.with_label(
            Label::new((
                &file_id,
                signature_change.new.source_range.start_byte
                    ..signature_change.new.source_range.end_byte,
            ))
            .with_message(label),
        );

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
