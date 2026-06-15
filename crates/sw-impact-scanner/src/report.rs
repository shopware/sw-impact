//! breaking change detection and its types

use std::fmt::Display;

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
    pub base_surface: Surface,
    pub kind: SignatureChangeKind,
    pub old: Option<SourceToken>,
    pub new: SourceToken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SignatureChangeKind {
    TsMethodParamRemoved,
    TsMethodParamTypeChanged,
    TsMethodParamRestChanged,
    TsMethodParamBecameRequired,
    TsMethodRequiredParamAdded,
    TsMethodReturnTypeChanged,
    TsMethodAsyncChanged,
}

impl Display for SignatureChangeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SignatureChangeKind::TsMethodParamRemoved => write!(f, "ts:method:param:removed"),
            SignatureChangeKind::TsMethodParamTypeChanged => {
                write!(f, "ts:method:param:type:changed")
            }
            SignatureChangeKind::TsMethodParamRestChanged => {
                write!(f, "ts:method:param:rest:changed")
            }
            SignatureChangeKind::TsMethodParamBecameRequired => {
                write!(f, "ts:method:param:became:required")
            }
            SignatureChangeKind::TsMethodRequiredParamAdded => {
                write!(f, "ts:method:required:param:added")
            }
            SignatureChangeKind::TsMethodReturnTypeChanged => {
                write!(f, "ts:method:return:type:changed")
            }
            SignatureChangeKind::TsMethodAsyncChanged => write!(f, "ts:method:async:changed"),
        }
    }
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
                self.diff_ts_method(base, base_method, new, new_method)
            }
            (Signature::VueProp(base_prop), Signature::VueProp(new_prop)) => {
                self.diff_vue_prop(base, base_prop, new, new_prop)
            }
            _ => unreachable!("signatures are asserted to always be the same for the same FQN"),
        }
    }

    fn diff_ts_method(
        &mut self,
        base_surface: &Surface,
        base: &TsMethod,
        new_surface: &Surface,
        new: &TsMethod,
    ) {
        if base.r#async != new.r#async {
            self.breaking_changes.push(SignatureChange {
                base_surface: base_surface.clone(),
                kind: SignatureChangeKind::TsMethodAsyncChanged,
                old: base.r#async.clone(),
                new: match &new.r#async {
                    Some(t) => t.clone(),
                    None => new_surface.source_token.clone(),
                },
            });
        }

        if base.return_type != new.return_type {
            self.breaking_changes.push(SignatureChange {
                base_surface: base_surface.clone(),
                kind: SignatureChangeKind::TsMethodReturnTypeChanged,
                old: base.return_type.clone(),
                new: match &new.return_type {
                    Some(t) => t.clone(),
                    None => new_surface.source_token.clone(),
                },
            });

            return;
        }

        // check each parameter
        for (base_param, new_param) in base.parameters.iter().zip(new.parameters.iter()) {
            if base_param.optional != new_param.optional && new_param.default_value.is_none() {
                self.breaking_changes.push(SignatureChange {
                    base_surface: base_surface.clone(),
                    kind: SignatureChangeKind::TsMethodParamBecameRequired,
                    old: base_param.optional.clone(),
                    new: match &new_param.optional {
                        Some(t) => t.clone(),
                        None => new_param.name.clone(),
                    },
                });
            }

            if base_param.type_annotation != new_param.type_annotation {
                self.breaking_changes.push(SignatureChange {
                    base_surface: base_surface.clone(),
                    kind: SignatureChangeKind::TsMethodParamTypeChanged,
                    old: base_param.type_annotation.clone(),
                    new: match &new_param.type_annotation {
                        Some(t) => t.clone(),
                        None => new_param.name.clone(),
                    },
                });
            }

            if base_param.rest != new_param.rest {
                self.breaking_changes.push(SignatureChange {
                    base_surface: base_surface.clone(),
                    kind: SignatureChangeKind::TsMethodParamRestChanged,
                    old: base_param.rest.clone(),
                    new: match &new_param.rest {
                        Some(t) => t.clone(),
                        None => new_param.name.clone(),
                    },
                });
            }
        }

        // check for added required parameter
        for i in base.parameters.len()..new.parameters.len() {
            let param = &new.parameters[i];

            if param.optional.is_none() && param.default_value.is_none() {
                self.breaking_changes.push(SignatureChange {
                    base_surface: base_surface.clone(),
                    kind: SignatureChangeKind::TsMethodRequiredParamAdded,
                    old: None,
                    new: param.name.clone(),
                });
            }
        }

        // check for removed parameter
        for i in new.parameters.len()..base.parameters.len() {
            let param = &base.parameters[i];

            self.breaking_changes.push(SignatureChange {
                base_surface: base_surface.clone(),
                kind: SignatureChangeKind::TsMethodParamRemoved,
                old: Some(param.name.clone()),
                new: new_surface.source_token.clone(),
            });
        }
    }

    fn diff_vue_prop(
        &mut self,
        base_surface: &Surface,
        base: &VueProp,
        new_surface: &Surface,
        new: &VueProp,
    ) {
        // TODO: implement me
    }
}
