use std::{collections::HashMap, path::PathBuf, sync::Mutex};

use tracing::error;

pub type SurfaceMap = HashMap<String, Surface>;

#[derive(Debug)]
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

#[derive(Debug)]
pub enum Signature {
    None,
    VueMethod(VueMethod),
}

#[derive(Debug)]
pub struct VueMethod {
    pub parameters: String, // TODO: proper parameter parsing
    pub return_type: Option<String>,
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
