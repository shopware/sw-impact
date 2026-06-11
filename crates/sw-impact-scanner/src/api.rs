//! public API surface types and utility methods

use std::{
    collections::HashMap,
    error::Error,
    fs::File,
    io::{BufReader, BufWriter},
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::{Deserialize, Serialize};
use tracing::{error, trace};

pub type SurfaceMap = HashMap<String, Surface>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Surface {
    pub file_path: PathBuf,
    pub source_range: SourceRange,
    pub fqn: String,
    pub signature: Signature,
}

impl Surface {
    pub fn builder() -> SurfaceBuilder {
        SurfaceBuilder::new()
    }
}

#[derive(Debug, Default)]
pub struct SurfaceBuilder {
    file_path: Option<PathBuf>,
    source_range: Option<SourceRange>,
    fqn: Option<String>,
    signature: Option<Signature>,
}

impl SurfaceBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn build(self) -> Result<Surface, &'static str> {
        Ok(Surface {
            file_path: self.file_path.ok_or("file_path required")?,
            source_range: self.source_range.ok_or("source_range required")?,
            fqn: self.fqn.ok_or("fqn required")?,
            signature: self.signature.unwrap_or(Signature::None),
        })
    }

    pub fn file_path(mut self, file_path: impl Into<PathBuf>) -> Self {
        self.file_path = Some(file_path.into());
        self
    }

    pub fn source_range(mut self, source_range: impl Into<SourceRange>) -> Self {
        self.source_range = Some(source_range.into());
        self
    }

    pub fn fqn(mut self, fqn: impl Into<String>) -> Self {
        self.fqn = Some(fqn.into());
        self
    }

    pub fn signature(mut self, signature: impl Into<Signature>) -> Self {
        self.signature = Some(signature.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Signature {
    None,
    TsMethod(TsMethod),
    VueProp(VueProp),
}

impl From<TsMethod> for Signature {
    fn from(value: TsMethod) -> Self {
        Signature::TsMethod(value)
    }
}

impl From<VueProp> for Signature {
    fn from(value: VueProp) -> Self {
        Signature::VueProp(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TsMethod {
    pub is_async: bool,
    pub parameters: Vec<TsParam>,
    pub return_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VueProp {
    pub definition: String, // TODO: proper parsing
}

/// Typescript / Javascript function declaration parameter
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TsParam {
    pub name: String,
    /// e.g. 'foo: string'
    pub type_annotation: Option<String>,
    /// e.g. 'foo = 1'
    pub default_value: Option<String>,
    /// e.g. 'foo?'
    pub is_optional: bool,
    /// e.g. '...args'
    pub is_rest: bool,
}

#[derive(Debug)]
pub struct SurfaceCollector {
    map: Mutex<SurfaceMap>,
}

impl SurfaceCollector {
    pub fn new() -> Self {
        Self {
            map: Default::default(),
        }
    }

    pub fn push(&self, surface: Surface) {
        trace!("adding fqn: {}", surface.fqn);
        let updated = self
            .map
            .lock()
            .unwrap()
            .insert(surface.fqn.clone(), surface);
        if let Some(updated) = updated {
            error!("duplicate FQN: {}", updated.fqn);
        }
    }

    pub fn finish(self) -> SurfaceMap {
        self.map.into_inner().unwrap()
    }
}

impl Default for SurfaceCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// A range of positions in a multi-line text document, both in terms of bytes and of rows and columns.
/// Based on [tree_sitter::Range]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceRange {
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_point: SourcePoint,
    pub end_point: SourcePoint,
}

impl From<tree_sitter::Range> for SourceRange {
    fn from(value: tree_sitter::Range) -> Self {
        Self {
            start_byte: value.start_byte,
            end_byte: value.end_byte,
            start_point: value.start_point.into(),
            end_point: value.end_point.into(),
        }
    }
}

/// A position in a multi-line text document, in terms of rows and columns.
/// Rows and columns are zero-based.
/// Based on [tree_sitter::Point]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourcePoint {
    pub row: usize,
    pub column: usize,
}

impl From<tree_sitter::Point> for SourcePoint {
    fn from(value: tree_sitter::Point) -> Self {
        Self {
            row: value.row,
            column: value.column,
        }
    }
}

// TODO: add proper error type
pub fn save_surface_map(
    surface_map: &SurfaceMap,
    path: impl AsRef<Path>,
) -> Result<(), Box<dyn Error>> {
    let file = File::create(path)?;
    let writer = BufWriter::new(file);
    serde_json::to_writer_pretty(writer, surface_map)?;
    Ok(())
}

// TODO: add proper error type
pub fn load_surface_map(path: impl AsRef<Path>) -> Result<SurfaceMap, Box<dyn Error>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    Ok(serde_json::from_reader(reader)?)
}
