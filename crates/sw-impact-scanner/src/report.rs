//! breaking change detection and its types

use serde::Serialize;

use crate::api::{Signature, SourceToken, Surface, SurfaceMap, TsMethod, VueProp};

#[derive(Debug, Clone, Default, Serialize)]
pub struct Report {
    /// removed public API surfaces that existed in the base index
    pub removed: Vec<Surface>,
    /// newly introduced public API surfaces, just nice to know
    //pub introduced: Vec<Surface>,

    /// all detected potential breaking changes
    pub breaking_changes: Vec<SignatureChange>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SignatureChange {
    pub surface: Surface,
    pub kind: SignatureChangeKind,
    pub old: Option<SourceToken>,
    pub new: Option<SourceToken>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SignatureChangeKind {
    TsMethodParamRemoved,
    TsMethodParamTypeChanged,
    TsMethodRequiredParamAdded,
    TsMethodReturnTypeChanged,
    TsMethodAsyncChanged,
}

impl Report {
    pub fn build_report(base: &SurfaceMap, new: &SurfaceMap) -> Self {
        let mut report = Self::default();

        for (fqn, base_surface) in base {
            if let Some(new_surface) = new.get(fqn) {
                report.diff_surfaces(base_surface, new_surface);
            } else {
                report.removed.push(base_surface.clone());
            }
        }

        report
    }

    pub fn contains_breaks(&self) -> bool {
        !self.removed.is_empty() || !self.breaking_changes.is_empty()
    }

    fn diff_surfaces(&mut self, base: &Surface, new: &Surface) {
        debug_assert_eq!(
            base.fqn, new.fqn,
            "expected to be called with two Surfaces of same fqn, but got {} and {}",
            base.fqn, new.fqn
        );
        assert_eq!(
            std::mem::discriminant(&base.signature),
            std::mem::discriminant(&new.signature),
            "expected same signatures for {}, got {:?} and {:?}",
            base.fqn,
            base.signature,
            new.signature,
        );

        match (&base.signature, &new.signature) {
            (Signature::None, Signature::None) => {}
            (Signature::TsMethod(base_method), Signature::TsMethod(new_method)) => {
                self.diff_ts_method(new, base_method, new_method)
            }
            (Signature::VueProp(base_prop), Signature::VueProp(new_prop)) => {
                self.diff_vue_prop(new, base_prop, new_prop)
            }
            _ => unreachable!("signatures are asserted to always be the same for the same FQN"),
        }
    }

    fn diff_ts_method(&mut self, surface: &Surface, base: &TsMethod, new: &TsMethod) {
        if base.r#async != new.r#async {
            self.breaking_changes.push(SignatureChange {
                surface: surface.clone(),
                kind: SignatureChangeKind::TsMethodAsyncChanged,
                old: base.r#async.clone(),
                new: new.r#async.clone(),
            });
        }

        if base.return_type != new.return_type {
            self.breaking_changes.push(SignatureChange {
                surface: surface.clone(),
                kind: SignatureChangeKind::TsMethodReturnTypeChanged,
                old: base.return_type.clone(),
                new: new.return_type.clone(),
            });

            return;
        }

        // check for added required parameter
        if new.parameters.len() > base.parameters.len() {
            for i in base.parameters.len()..new.parameters.len() {
                let param = &new.parameters[i];

                if param.optional.is_none() && param.default_value.is_none() {
                    self.breaking_changes.push(SignatureChange {
                        surface: surface.clone(),
                        kind: SignatureChangeKind::TsMethodRequiredParamAdded,
                        old: None,
                        new: Some(param.name.clone()),
                    });
                }
            }
        }

        // check for removed parameter
        if base.parameters.len() > new.parameters.len() {
            for i in new.parameters.len()..base.parameters.len() {
                let param = &base.parameters[i];

                self.breaking_changes.push(SignatureChange {
                    surface: surface.clone(),
                    kind: SignatureChangeKind::TsMethodParamRemoved,
                    old: Some(param.name.clone()),
                    new: None,
                });
            }
        }

        // TODO: compare each parameter for breaking changes
    }

    fn diff_vue_prop(&mut self, surface: &Surface, base: &VueProp, new: &VueProp) {
        // TODO: implement me
    }
}
