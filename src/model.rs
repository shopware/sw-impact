use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SurfaceKey(String);

impl SurfaceKey {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn kind(&self) -> SurfaceKind {
        SurfaceKind::from_key(&self.0)
    }
}

impl fmt::Display for SurfaceKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SurfaceKind {
    PhpClass,
    PhpMethod,
    PhpProperty,
    PhpConst,
    ServiceId,
    EventClass,
    EventName,
    RouteName,
    ApiRoute,
    DalEntity,
    DalField,
    TwigTemplate,
    TwigBlock,
    AdminComponent,
    AdminRoute,
    AdminStateStore,
    StorefrontJsPlugin,
    FeatureFlag,
    Unknown,
}

impl SurfaceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PhpClass => "php-class",
            Self::PhpMethod => "php-method",
            Self::PhpProperty => "php-property",
            Self::PhpConst => "php-const",
            Self::ServiceId => "service-id",
            Self::EventClass => "event-class",
            Self::EventName => "event-name",
            Self::RouteName => "route-name",
            Self::ApiRoute => "api-route",
            Self::DalEntity => "dal-entity",
            Self::DalField => "dal-field",
            Self::TwigTemplate => "twig-template",
            Self::TwigBlock => "twig-block",
            Self::AdminComponent => "admin-component",
            Self::AdminRoute => "admin-route",
            Self::AdminStateStore => "admin-state-store",
            Self::StorefrontJsPlugin => "storefront-js-plugin",
            Self::FeatureFlag => "feature-flag",
            Self::Unknown => "unknown",
        }
    }

    pub fn from_key(key: &str) -> Self {
        if key.starts_with("php:class:") {
            Self::PhpClass
        } else if key.starts_with("php:method:") {
            Self::PhpMethod
        } else if key.starts_with("php:property:") {
            Self::PhpProperty
        } else if key.starts_with("php:const:") {
            Self::PhpConst
        } else if key.starts_with("service:id:") {
            Self::ServiceId
        } else if key.starts_with("event:class:") {
            Self::EventClass
        } else if key.starts_with("event:name:") {
            Self::EventName
        } else if key.starts_with("route:name:") {
            Self::RouteName
        } else if key.starts_with("api:") {
            Self::ApiRoute
        } else if key.starts_with("dal:entity:") {
            Self::DalEntity
        } else if key.starts_with("dal:field:") {
            Self::DalField
        } else if key.starts_with("twig:template:") {
            Self::TwigTemplate
        } else if key.starts_with("twig:block:") {
            Self::TwigBlock
        } else if key.starts_with("admin:component:") {
            Self::AdminComponent
        } else if key.starts_with("admin:route:") {
            Self::AdminRoute
        } else if key.starts_with("admin:state-store:") {
            Self::AdminStateStore
        } else if key.starts_with("storefront-js-plugin:") {
            Self::StorefrontJsPlugin
        } else if key.starts_with("feature-flag:") {
            Self::FeatureFlag
        } else {
            Self::Unknown
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Confidence {
    Low = 1,
    Medium = 2,
    High = 3,
}

impl Confidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    pub fn from_i64(value: i64) -> Self {
        match value {
            3 => Self::High,
            2 => Self::Medium,
            _ => Self::Low,
        }
    }

    pub fn include(self, include_low: bool, only_high: bool) -> bool {
        if only_high {
            return self == Self::High;
        }

        include_low || self != Self::Low
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FactRole {
    Usage,
    Definition,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Evidence {
    pub path: PathBuf,
    pub line: usize,
    pub column: Option<usize>,
    pub snippet: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Signature {
    pub visibility: Option<String>,
    pub parameters: Vec<Parameter>,
    pub return_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Parameter {
    pub name: String,
    pub type_name: Option<String>,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Fact {
    pub surface: SurfaceKey,
    pub kind: SurfaceKind,
    pub role: FactRole,
    pub evidence: Evidence,
    pub confidence: Confidence,
    pub usage_kind: String,
    pub signature: Option<Signature>,
}

impl Fact {
    pub fn usage(
        surface: impl Into<String>,
        evidence: Evidence,
        confidence: Confidence,
        usage_kind: impl Into<String>,
    ) -> Self {
        let surface = SurfaceKey::new(surface);
        let kind = surface.kind();

        Self {
            surface,
            kind,
            role: FactRole::Usage,
            evidence,
            confidence,
            usage_kind: usage_kind.into(),
            signature: None,
        }
    }

    pub fn definition(
        surface: impl Into<String>,
        evidence: Evidence,
        confidence: Confidence,
        usage_kind: impl Into<String>,
        signature: Option<Signature>,
    ) -> Self {
        let surface = SurfaceKey::new(surface);
        let kind = surface.kind();

        Self {
            surface,
            kind,
            role: FactRole::Definition,
            evidence,
            confidence,
            usage_kind: usage_kind.into(),
            signature,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeKind {
    Removed,
    PossibleRename,
    SignatureChanged,
    VisibilityChanged,
    RequiredParameterAdded,
    ParameterRemoved,
    ReturnTypeChanged,
    DefinitionAdded,
}

impl ChangeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Removed => "removed",
            Self::PossibleRename => "possible rename",
            Self::SignatureChanged => "signature changed",
            Self::VisibilityChanged => "visibility changed",
            Self::RequiredParameterAdded => "required parameter added",
            Self::ParameterRemoved => "parameter removed",
            Self::ReturnTypeChanged => "return type changed",
            Self::DefinitionAdded => "definition added",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChangedSurface {
    pub surface: SurfaceKey,
    pub kind: SurfaceKind,
    pub change: ChangeKind,
    pub confidence: Confidence,
    pub evidence: Evidence,
}

pub fn line_col_at(content: &str, byte_offset: usize) -> (usize, usize) {
    let mut line = 1;
    let mut col = 1;

    for (index, ch) in content.char_indices() {
        if index >= byte_offset {
            break;
        }

        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }

    (line, col)
}

pub fn line_snippet(content: &str, line: usize) -> Option<String> {
    content
        .lines()
        .nth(line.saturating_sub(1))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| line.chars().take(240).collect())
}

#[derive(Debug, Clone)]
pub struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    pub fn new(content: &str) -> Self {
        let mut starts = vec![0];

        for (index, byte) in content.bytes().enumerate() {
            if byte == b'\n' && index + 1 < content.len() {
                starts.push(index + 1);
            }
        }

        Self { starts }
    }

    pub fn line_col(&self, content: &str, byte_offset: usize) -> (usize, usize) {
        let offset = previous_char_boundary(content, byte_offset.min(content.len()));
        let line_index = self
            .starts
            .partition_point(|line_start| *line_start <= offset)
            .saturating_sub(1);
        let line_start = self.starts.get(line_index).copied().unwrap_or(0);
        let column = content[line_start..offset].chars().count() + 1;

        (line_index + 1, column)
    }

    pub fn snippet(&self, content: &str, line: usize) -> Option<String> {
        let start = *self.starts.get(line.checked_sub(1)?)?;
        let end = self
            .starts
            .get(line)
            .copied()
            .map(|next_start| next_start.saturating_sub(1))
            .unwrap_or(content.len());

        content
            .get(start..end)
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| line.chars().take(240).collect())
    }
}

fn previous_char_boundary(content: &str, mut offset: usize) -> usize {
    offset = offset.min(content.len());

    while offset > 0 && !content.is_char_boundary(offset) {
        offset -= 1;
    }

    offset
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_surface_kind_from_key() {
        assert_eq!(
            SurfaceKey::new("php:method:Shopware\\Core\\Foo::bar").kind(),
            SurfaceKind::PhpMethod
        );
        assert_eq!(
            SurfaceKey::new("twig:block:storefront_page").kind(),
            SurfaceKind::TwigBlock
        );
        assert_eq!(
            SurfaceKey::new("admin:component:sw-product-detail").kind(),
            SurfaceKind::AdminComponent
        );
    }

    #[test]
    fn filters_confidence_for_default_and_high_only_modes() {
        assert!(Confidence::High.include(false, false));
        assert!(Confidence::Medium.include(false, false));
        assert!(!Confidence::Low.include(false, false));
        assert!(Confidence::Low.include(true, false));
        assert!(!Confidence::Medium.include(true, true));
        assert!(Confidence::High.include(true, true));
    }

    #[test]
    fn computes_line_column_and_snippet() {
        let content = "first\n  second line\nthird";
        let offset = content.find("second").expect("fixture contains second");

        assert_eq!(line_col_at(content, offset), (2, 3));
        assert_eq!(line_snippet(content, 2).as_deref(), Some("second line"));
    }

    #[test]
    fn line_index_handles_non_ascii_offsets() {
        let content = "für\n  second line\nthird";
        let offset = content.find("second").expect("fixture contains second");
        let index = LineIndex::new(content);

        assert_eq!(index.line_col(content, offset), (2, 3));
        assert_eq!(index.snippet(content, 2).as_deref(), Some("second line"));
    }
}
