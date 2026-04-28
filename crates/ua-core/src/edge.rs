use serde::{Deserialize, Serialize};

/// 35 edge types in 8 categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeType {
    // Structural
    Imports,
    Exports,
    Contains,
    Inherits,
    Implements,
    // Behavioral
    Calls,
    Subscribes,
    Publishes,
    Middleware,
    // Data flow
    ReadsFrom,
    WritesTo,
    Transforms,
    Validates,
    // Dependencies
    DependsOn,
    TestedBy,
    Configures,
    // Semantic
    Related,
    SimilarTo,
    // Infrastructure
    Deploys,
    Serves,
    Provisions,
    Triggers,
    // Schema/Data
    Migrates,
    Documents,
    Routes,
    DefinesSchema,
    // Domain
    ContainsFlow,
    FlowStep,
    CrossDomain,
    // Knowledge
    Cites,
    Contradicts,
    BuildsOn,
    Exemplifies,
    CategorizedUnder,
    AuthoredBy,
}

impl EdgeType {
    pub const ALL: [EdgeType; 35] = [
        EdgeType::Imports,
        EdgeType::Exports,
        EdgeType::Contains,
        EdgeType::Inherits,
        EdgeType::Implements,
        EdgeType::Calls,
        EdgeType::Subscribes,
        EdgeType::Publishes,
        EdgeType::Middleware,
        EdgeType::ReadsFrom,
        EdgeType::WritesTo,
        EdgeType::Transforms,
        EdgeType::Validates,
        EdgeType::DependsOn,
        EdgeType::TestedBy,
        EdgeType::Configures,
        EdgeType::Related,
        EdgeType::SimilarTo,
        EdgeType::Deploys,
        EdgeType::Serves,
        EdgeType::Provisions,
        EdgeType::Triggers,
        EdgeType::Migrates,
        EdgeType::Documents,
        EdgeType::Routes,
        EdgeType::DefinesSchema,
        EdgeType::ContainsFlow,
        EdgeType::FlowStep,
        EdgeType::CrossDomain,
        EdgeType::Cites,
        EdgeType::Contradicts,
        EdgeType::BuildsOn,
        EdgeType::Exemplifies,
        EdgeType::CategorizedUnder,
        EdgeType::AuthoredBy,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EdgeDirection {
    Forward,
    Backward,
    Bidirectional,
}

/// A single graph edge — wire-compatible with the original `GraphEdge`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
    #[serde(rename = "type")]
    pub edge_type: EdgeType,
    pub direction: EdgeDirection,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub weight: f32,
}
