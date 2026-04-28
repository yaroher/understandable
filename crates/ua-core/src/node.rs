use serde::{Deserialize, Serialize};

/// 21 node types — preserves the exact wire form used by the original.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeType {
    File,
    Function,
    Class,
    Module,
    Concept,
    Config,
    Document,
    Service,
    Table,
    Endpoint,
    Pipeline,
    Schema,
    Resource,
    Domain,
    Flow,
    Step,
    Article,
    Entity,
    Topic,
    Claim,
    Source,
}

impl NodeType {
    pub const ALL: [NodeType; 21] = [
        NodeType::File,
        NodeType::Function,
        NodeType::Class,
        NodeType::Module,
        NodeType::Concept,
        NodeType::Config,
        NodeType::Document,
        NodeType::Service,
        NodeType::Table,
        NodeType::Endpoint,
        NodeType::Pipeline,
        NodeType::Schema,
        NodeType::Resource,
        NodeType::Domain,
        NodeType::Flow,
        NodeType::Step,
        NodeType::Article,
        NodeType::Entity,
        NodeType::Topic,
        NodeType::Claim,
        NodeType::Source,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            NodeType::File => "file",
            NodeType::Function => "function",
            NodeType::Class => "class",
            NodeType::Module => "module",
            NodeType::Concept => "concept",
            NodeType::Config => "config",
            NodeType::Document => "document",
            NodeType::Service => "service",
            NodeType::Table => "table",
            NodeType::Endpoint => "endpoint",
            NodeType::Pipeline => "pipeline",
            NodeType::Schema => "schema",
            NodeType::Resource => "resource",
            NodeType::Domain => "domain",
            NodeType::Flow => "flow",
            NodeType::Step => "step",
            NodeType::Article => "article",
            NodeType::Entity => "entity",
            NodeType::Topic => "topic",
            NodeType::Claim => "claim",
            NodeType::Source => "source",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Complexity {
    Simple,
    Moderate,
    Complex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DomainEntryType {
    Http,
    Cli,
    Event,
    Cron,
    Manual,
}

/// Domain metadata for `domain`/`flow`/`step` nodes.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DomainMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entities: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub business_rules: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cross_domain_interactions: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_point: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_type: Option<DomainEntryType>,
}

/// Knowledge metadata for `article`/`entity`/`topic`/`claim`/`source` nodes.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wikilinks: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backlinks: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

/// A single graph node — wire-compatible with the original `GraphNode`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GraphNode {
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: NodeType,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_range: Option<(u32, u32)>,
    pub summary: String,
    pub tags: Vec<String>,
    pub complexity: Complexity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language_notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain_meta: Option<DomainMeta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub knowledge_meta: Option<KnowledgeMeta>,
}
