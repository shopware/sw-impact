use std::path::Path;

use sw_impact_scanner::{
    api::SourceRange,
    report::{Report, SignatureChangeKind},
};

pub fn output_github(report: &Report) {
    for surface in &report.removed {
        print_error(
            surface.file_path.as_path(),
            &surface.source_token.source_range,
            "api:removed",
            &format!(
                "Removed public API surface '{}'. Old signature was: '{}'",
                surface.fqn, surface.source_token.text_normalized
            ),
        );
    }

    for signature_change in &report.breaking_changes {
        let base_surface_text = &signature_change.base_surface.source_token.text_normalized;
        let old_text = signature_change
            .old
            .as_ref()
            .map(|t| t.text_normalized.as_str())
            .unwrap_or("");
        let detail = match signature_change.kind {
            SignatureChangeKind::TsMethodParamRemoved => {
                &format!("parameter '{old_text}' was removed")
            }
            SignatureChangeKind::TsMethodParamTypeChanged => &format!(
                "parameter type changed from '{old_text}' to '{}'",
                signature_change.new.text_normalized
            ),
            SignatureChangeKind::TsMethodParamRestChanged => "parameter ...rest changed",
            SignatureChangeKind::TsMethodParamBecameRequired => "parameter became required",
            SignatureChangeKind::TsMethodRequiredParamAdded => "required parameter was added",
            SignatureChangeKind::TsMethodReturnTypeChanged => &format!(
                "return type changed from '{old_text}' to '{}'",
                signature_change.new.text_normalized
            ),
            SignatureChangeKind::TsMethodAsyncChanged => "async changed",
            SignatureChangeKind::VuePropTypeChanged => &format!(
                "prop '{base_surface_text}' type changed from '{old_text}' to '{}'",
                signature_change.new.text_normalized
            ),
            SignatureChangeKind::VuePropBecameRequired => {
                &format!("prop '{base_surface_text}' became required")
            }
        };

        let mut msg = format!(
            "Potential breaking change on public API surface '{}': {detail}",
            signature_change.base_surface.fqn,
        );
        if !matches!(
            signature_change.kind,
            SignatureChangeKind::VuePropBecameRequired | SignatureChangeKind::VuePropTypeChanged
        ) {
            msg.push_str(&format!(
                " Old signature was: '{}'",
                signature_change.base_surface.source_token.text_normalized
            ));
        }

        print_error(
            signature_change.base_surface.file_path.as_path(),
            &signature_change.new.source_range,
            &signature_change.kind.to_string(),
            &msg,
        );
    }
}

fn print_error(file: &Path, range: &SourceRange, title: &str, message: &str) {
    println!(
        "::error file={},line={},col={},endLine={},endColumn={},title={}::{}",
        escape_property(&file.display().to_string()),
        range.start_point.row + 1,
        range.start_point.column + 1,
        range.end_point.row + 1,
        range.end_point.column + 1,
        escape_property(title),
        escape_data(message),
    );
}

fn escape_property(value: &str) -> String {
    escape_data(value).replace(':', "%3A").replace(',', "%2C")
}

fn escape_data(value: &str) -> String {
    value
        .replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}
