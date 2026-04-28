use serde::{Deserialize, Serialize};

use crate::edge::GraphEdge;
use crate::meta::ProjectMeta;
use crate::node::GraphNode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GraphKind {
    Codebase,
    Knowledge,
    Domain,
}

/// Logical grouping of nodes (e.g. API/Service/Data architectural layers).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Layer {
    pub id: String,
    pub name: String,
    pub description: String,
    pub node_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TourStep {
    pub order: u32,
    pub title: String,
    pub description: String,
    pub node_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language_lesson: Option<String>,
}

/// Top-level graph document — wire-compatible with the original.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KnowledgeGraph {
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<GraphKind>,
    pub project: ProjectMeta,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub layers: Vec<Layer>,
    pub tour: Vec<TourStep>,
}

impl KnowledgeGraph {
    pub fn new(project: ProjectMeta) -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            kind: Some(GraphKind::Codebase),
            project,
            nodes: Vec::new(),
            edges: Vec::new(),
            layers: Vec::new(),
            tour: Vec::new(),
        }
    }
}
