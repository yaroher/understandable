//! Core types and schema for the `understandable` knowledge graph.
//!
//! Mirrors the original `@understandable/core` `types.ts` shape so JSON
//! produced by the original tool round-trips into and out of these types.

pub mod edge;
pub mod error;
pub mod graph;
pub mod meta;
pub mod node;
pub mod plugin;
pub mod settings;
pub mod validate;

pub use edge::{EdgeDirection, EdgeType, GraphEdge};
pub use error::Error;
pub use graph::{GraphKind, KnowledgeGraph, Layer, TourStep};
pub use meta::{AnalysisMeta, ProjectConfig, ProjectMeta, ThemeConfig};
pub use node::{Complexity, DomainEntryType, DomainMeta, GraphNode, KnowledgeMeta, NodeType};
pub use plugin::{
    CallGraphEntry, ClassDecl, DefinitionInfo, EndpointInfo, ExportDecl, FunctionDecl, ImportDecl,
    ImportResolution, ReferenceResolution, ResourceInfo, SectionInfo, ServiceInfo, StepInfo,
    StructuralAnalysis,
};
pub use settings::{
    DashboardSettings, EmbeddingSettings, GitSettings, IgnoreSettings, IncrementalSettings,
    LlmSettings, ProjectIdent, ProjectSettings, StorageSettings,
};
pub use validate::{
    validate_graph, ComplexityHistogram, Severity, ValidationError, ValidationIssue,
    ValidationReport, ValidationStats,
};

/// Crate-level result alias.
pub type Result<T> = std::result::Result<T, Error>;

/// Deprecated alias retained for one release. New code should depend on
/// [`Error`] directly.
#[deprecated(since = "0.2.0", note = "use `ua_core::Error` instead")]
pub type SettingsError = Error;
