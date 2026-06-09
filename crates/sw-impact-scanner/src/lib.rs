use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

use ignore::{DirEntry, WalkBuilder, WalkState};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use thiserror::Error;
use tracing::info;

use crate::{
    api::{SurfaceCollector, SurfaceMap},
    vue::{process_file_vue, validate_queries as validate_vue_queries},
};

#[derive(Debug, Error)]
pub enum ScanError {}

pub mod api;
pub mod vue;

pub fn validate_queries() {
    validate_vue_queries();
}

pub fn scan_dir(path: &Path) -> Result<SurfaceMap, ScanError> {
    validate_queries();
    let collector = SurfaceCollector::new();

    let files = collect_files(path);

    files.par_iter().for_each(|path| {
        process_file(path, &collector);
    });

    let surface_map = collector.finish();

    info!("surfaces found: {}", surface_map.len());

    Ok(surface_map)
}

fn process_file(path: &Path, collector: &SurfaceCollector) {
    let file_ext = path.extension().and_then(|e| e.to_str());
    match file_ext {
        Some("js") | Some("ts") => {
            if is_administration_path(path) {
                process_file_vue(path, collector)
            } else {
                // TODO: storefront
            }
        }
        Some("twig") => {} // TODO: implement
        Some("php") => {}  // TODO: implement
        _ => unreachable!("process unknown file extension"),
    }
}

fn collect_files(path: &Path) -> Vec<PathBuf> {
    // use parallel iteration because its much faster
    // downside is it needs some multithreading wrappers

    let files = Mutex::new(Vec::new());
    let walker = WalkBuilder::new(path)
        .filter_entry(file_entry_filter)
        .build_parallel();
    walker.run(|| {
        Box::new(|maybe_entry| {
            let Ok(entry) = maybe_entry else {
                return WalkState::Continue;
            };

            if let Some(file_type) = entry.file_type()
                && file_type.is_dir()
            {
                return WalkState::Continue;
            }

            files.lock().unwrap().push(entry.into_path());
            WalkState::Continue
        })
    });

    files.into_inner().unwrap()
}

/// returns true for all files and directories it should visit
fn file_entry_filter(entry: &DirEntry) -> bool {
    let file_name = entry.file_name().to_string_lossy();
    if let Some(file_type) = entry.file_type()
        && file_type.is_dir()
    {
        return match file_name.as_ref() {
            // don't traverse in any folder with these names
            "tests" | "test" => false,
            _ => true,
        };
    }

    let Some((file_stem, file_ext)) = file_name.rsplit_once('.') else {
        return false;
    };

    if file_stem.ends_with(".spec") {
        return false;
    }

    matches!(file_ext, "php" | "js" | "ts" | "twig")
}

fn is_administration_path(path: &Path) -> bool {
    let mut iter = path.components();

    while let Some(p) = iter.next()
        && p.as_os_str() != "Resources"
    {
        // skip
    }

    if iter.next().is_none_or(|p| p.as_os_str() != "app") {
        return false;
    }

    if iter
        .next()
        .is_none_or(|p| p.as_os_str() != "administration")
    {
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_administration_path() {
        assert!(is_administration_path(Path::new(
            "Resources/app/administration/src/main.js"
        )));
        assert!(is_administration_path(Path::new(
            "custom/plugins/Foo/src/Resources/app/administration/src/module/foo/index.js"
        )));

        // should not match
        assert!(!is_administration_path(Path::new(
            "app/administration/src/main.js"
        )));
        assert!(!is_administration_path(Path::new(
            "src/Storefront/Resources/app/storefront/src/main.js"
        )));
    }
}
