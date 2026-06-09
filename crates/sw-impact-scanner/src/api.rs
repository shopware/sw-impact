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

#[derive(Debug, Serialize, Deserialize)]
pub struct Surface {
    pub file_path: PathBuf,
    pub fqn: String,
    pub signature: Signature,
}

impl Surface {
    pub fn new(file_path: PathBuf, fqn: String) -> Self {
        Self {
            file_path,
            fqn,
            signature: Signature::None,
        }
    }

    pub fn with_signature(file_path: PathBuf, fqn: String, signature: Signature) -> Self {
        Self {
            file_path,
            fqn,
            signature,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Signature {
    None,
    VueMethod(VueMethod),
    VueProp(VueProp),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VueMethod {
    pub parameters: String, // TODO: proper parameter parsing
    pub return_type: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VueProp {
    pub definition: String, // TODO: proper parsing
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
