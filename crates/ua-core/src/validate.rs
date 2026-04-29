//! Deterministic graph validation.
//!
//! Mirrors the TS inline validator that ships with `@understand-anything/skill`
//! (see `plugin/skills/understand/SKILL.md` lines 480-577). Output JSON shape:
//!
//! ```json
//! {
//!   "valid": true,
//!   "issues":   [{ "severity": "error", "code": "...", "message": "...", "nodeId": "...", "edgeId": "..." }],
//!   "warnings": [{ "severity": "warn",  "code": "...", "message": "...", "nodeId": "...", "edgeId": "..." }],
//!   "stats": {
//!     "totalNodes": 123, "totalEdges": 234, "totalLayers": 5, "totalTourSteps": 7,
//!     "nodeTypes": { "file": 50, ... }, "edgeTypes": { "imports": 80, ... },
//!     "complexityHistogram": { "simple": 10, "moderate": 80, "complex": 30 }
//!   }
//! }
//! ```
//!
//! `BTreeMap` is used for `nodeTypes`/`edgeTypes` so the JSON output is
//! key-sorted (deterministic).

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::graph::KnowledgeGraph;
use crate::node::{Complexity, NodeType};

// ---------------------------------------------------------------------------
// Legacy referential-integrity error enum.
//
// Retained verbatim so callers that still pattern-match against
// `report.errors` (e.g. the workspace `Error::Validation` variant, the
// roundtrip test) keep compiling. New callers should consume
// [`ValidationReport::issues`] / [`ValidationReport::warnings`] instead.
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("graph contains {0} duplicate node id(s); first: {1}")]
    DuplicateNodeId(usize, String),
    #[error("edge references unknown node id: {0}")]
    UnknownNodeRef(String),
    #[error("layer {layer} references unknown node id {node}")]
    LayerUnknownNode { layer: String, node: String },
    #[error("tour step {step} references unknown node id {node}")]
    TourUnknownNode { step: u32, node: String },
    #[error("edge weight out of [0,1]: {0}")]
    EdgeWeightOutOfRange(f32),
}

// ---------------------------------------------------------------------------
// New TS-shape report.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warn,
}

/// One validation issue (error or warning). Serialises as the TS shape:
/// `{ severity, code, message, nodeId?, edgeId? }`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ValidationIssue {
    pub severity: Severity,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ComplexityHistogram {
    pub simple: usize,
    pub moderate: usize,
    pub complex: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ValidationStats {
    pub total_nodes: usize,
    pub total_edges: usize,
    pub total_layers: usize,
    pub total_tour_steps: usize,
    pub node_types: BTreeMap<String, usize>,
    pub edge_types: BTreeMap<String, usize>,
    pub complexity_histogram: ComplexityHistogram,
}

/// Aggregated report mirroring the TS validator's output. `valid` is
/// `issues.is_empty()` — warnings do not fail the build by default.
///
/// `errors` is kept as a back-compat alias holding the same set of
/// fatal problems re-expressed as the legacy [`ValidationError`] enum.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationReport {
    pub valid: bool,
    pub issues: Vec<ValidationIssue>,
    pub warnings: Vec<ValidationIssue>,
    pub stats: ValidationStats,
    /// Legacy enum-typed view of `issues`, for callers that pattern-match.
    /// Skipped from JSON output.
    #[serde(skip)]
    pub errors: Vec<ValidationError>,
}

impl ValidationReport {
    /// True when no error-severity issues were found. Warnings do not
    /// flip this. `--strict` mode in the CLI is the place to fail on
    /// warnings.
    pub fn is_valid(&self) -> bool {
        self.issues.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Stable issue codes. These are part of the public CLI contract (the JSON
// output is consumed by humans and scripts), so don't rename casually.
// ---------------------------------------------------------------------------

/// Codes for validation issues. Stable identifiers — keep in sync with the
/// TS validator (loosely; TS uses free-form strings, we attach codes).
pub mod codes {
    // Errors.
    pub const DUPLICATE_NODE_ID: &str = "DUPLICATE_NODE_ID";
    pub const MISSING_SOURCE_NODE: &str = "MISSING_SOURCE_NODE";
    pub const MISSING_TARGET_NODE: &str = "MISSING_TARGET_NODE";
    pub const MISSING_NODE_REFERENCE: &str = "MISSING_NODE_REFERENCE";
    pub const INVALID_EDGE_WEIGHT: &str = "INVALID_EDGE_WEIGHT";
    pub const LAYER_MISSING_NODE: &str = "LAYER_MISSING_NODE";
    pub const TOUR_MISSING_NODE: &str = "TOUR_MISSING_NODE";

    // Warnings.
    pub const ORPHAN_NODE: &str = "ORPHAN_NODE";
    pub const FILE_NODE_NOT_IN_LAYER: &str = "FILE_NODE_NOT_IN_LAYER";
    pub const NODE_IN_MULTIPLE_LAYERS: &str = "NODE_IN_MULTIPLE_LAYERS";
    pub const MISSING_TAGS: &str = "MISSING_TAGS";
    pub const MISSING_SUMMARY: &str = "MISSING_SUMMARY";
}

/// Node types treated as "file-level" for the layer-coverage warning. Mirrors
/// the TS `fileLevelTypes` set in `plugin/skills/understand/SKILL.md:539`.
fn is_file_level(node_type: NodeType) -> bool {
    matches!(
        node_type,
        NodeType::File
            | NodeType::Config
            | NodeType::Document
            | NodeType::Service
            | NodeType::Pipeline
            | NodeType::Table
            | NodeType::Schema
            | NodeType::Resource
            | NodeType::Endpoint
    )
}

fn edge_type_str(t: crate::edge::EdgeType) -> &'static str {
    use crate::edge::EdgeType::*;
    match t {
        Imports => "imports",
        Exports => "exports",
        Contains => "contains",
        Inherits => "inherits",
        Implements => "implements",
        Calls => "calls",
        Subscribes => "subscribes",
        Publishes => "publishes",
        Middleware => "middleware",
        ReadsFrom => "reads_from",
        WritesTo => "writes_to",
        Transforms => "transforms",
        Validates => "validates",
        DependsOn => "depends_on",
        TestedBy => "tested_by",
        Configures => "configures",
        Related => "related",
        SimilarTo => "similar_to",
        Deploys => "deploys",
        Serves => "serves",
        Provisions => "provisions",
        Triggers => "triggers",
        Migrates => "migrates",
        Documents => "documents",
        Routes => "routes",
        DefinesSchema => "defines_schema",
        ContainsFlow => "contains_flow",
        FlowStep => "flow_step",
        CrossDomain => "cross_domain",
        Cites => "cites",
        Contradicts => "contradicts",
        BuildsOn => "builds_on",
        Exemplifies => "exemplifies",
        CategorizedUnder => "categorized_under",
        AuthoredBy => "authored_by",
    }
}

/// Run all referential-integrity, shape, and quality checks. Returns a
/// report listing every problem found rather than failing fast — matches
/// the `--review` agent's behaviour of reporting batches.
pub fn validate_graph(graph: &KnowledgeGraph) -> ValidationReport {
    let mut issues: Vec<ValidationIssue> = Vec::new();
    let mut warnings: Vec<ValidationIssue> = Vec::new();
    let mut errors: Vec<ValidationError> = Vec::new();

    // ---- duplicate ids ----------------------------------------------------
    let mut seen: HashSet<&str> = HashSet::with_capacity(graph.nodes.len());
    let mut dup_count = 0usize;
    let mut first_dup: Option<&str> = None;
    for node in &graph.nodes {
        if !seen.insert(node.id.as_str()) {
            dup_count += 1;
            first_dup.get_or_insert(node.id.as_str());
            issues.push(ValidationIssue {
                severity: Severity::Error,
                code: codes::DUPLICATE_NODE_ID.to_string(),
                message: format!("duplicate node id '{}'", node.id),
                node_id: Some(node.id.clone()),
                edge_id: None,
            });
        }
    }
    if dup_count > 0 {
        errors.push(ValidationError::DuplicateNodeId(
            dup_count,
            first_dup.unwrap_or("").to_string(),
        ));
    }

    let node_ids: HashSet<&str> = graph.nodes.iter().map(|n| n.id.as_str()).collect();

    // ---- per-node shape warnings -----------------------------------------
    for node in &graph.nodes {
        if node.tags.is_empty() {
            warnings.push(ValidationIssue {
                severity: Severity::Warn,
                code: codes::MISSING_TAGS.to_string(),
                message: format!("node '{}' has no tags", node.id),
                node_id: Some(node.id.clone()),
                edge_id: None,
            });
        }
        if node.summary.trim().is_empty() {
            warnings.push(ValidationIssue {
                severity: Severity::Warn,
                code: codes::MISSING_SUMMARY.to_string(),
                message: format!("node '{}' has empty summary", node.id),
                node_id: Some(node.id.clone()),
                edge_id: None,
            });
        }
    }

    // ---- edges ------------------------------------------------------------
    for (i, edge) in graph.edges.iter().enumerate() {
        let edge_id = format!("{}:{}->{}", i, edge.source, edge.target);
        if !node_ids.contains(edge.source.as_str()) {
            issues.push(ValidationIssue {
                severity: Severity::Error,
                code: codes::MISSING_SOURCE_NODE.to_string(),
                message: format!("edge[{i}] source '{}' not found among nodes", edge.source),
                node_id: Some(edge.source.clone()),
                edge_id: Some(edge_id.clone()),
            });
            errors.push(ValidationError::UnknownNodeRef(edge.source.clone()));
        }
        if !node_ids.contains(edge.target.as_str()) {
            issues.push(ValidationIssue {
                severity: Severity::Error,
                code: codes::MISSING_TARGET_NODE.to_string(),
                message: format!("edge[{i}] target '{}' not found among nodes", edge.target),
                node_id: Some(edge.target.clone()),
                edge_id: Some(edge_id.clone()),
            });
            errors.push(ValidationError::UnknownNodeRef(edge.target.clone()));
        }
        if !(0.0..=1.0).contains(&edge.weight) {
            issues.push(ValidationIssue {
                severity: Severity::Error,
                code: codes::INVALID_EDGE_WEIGHT.to_string(),
                message: format!("edge[{i}] weight {} out of [0,1]", edge.weight),
                node_id: None,
                edge_id: Some(edge_id),
            });
            errors.push(ValidationError::EdgeWeightOutOfRange(edge.weight));
        }
    }

    // ---- layers -----------------------------------------------------------
    // Track which layer(s) each node appears in, so we can flag both
    // multi-layer membership and missing-layer file nodes in one pass.
    let mut node_to_layers: HashMap<&str, Vec<&str>> = HashMap::new();
    for layer in &graph.layers {
        for nid in &layer.node_ids {
            if !node_ids.contains(nid.as_str()) {
                issues.push(ValidationIssue {
                    severity: Severity::Error,
                    code: codes::LAYER_MISSING_NODE.to_string(),
                    message: format!("layer '{}' references unknown node '{}'", layer.id, nid),
                    node_id: Some(nid.clone()),
                    edge_id: None,
                });
                errors.push(ValidationError::LayerUnknownNode {
                    layer: layer.id.clone(),
                    node: nid.clone(),
                });
            }
            node_to_layers
                .entry(nid.as_str())
                .or_default()
                .push(layer.id.as_str());
        }
    }
    // Multi-layer membership warning. Emit once per offending node.
    for (nid, layer_ids) in &node_to_layers {
        if layer_ids.len() >= 2 {
            warnings.push(ValidationIssue {
                severity: Severity::Warn,
                code: codes::NODE_IN_MULTIPLE_LAYERS.to_string(),
                message: format!(
                    "node '{}' appears in {} layers: {}",
                    nid,
                    layer_ids.len(),
                    layer_ids.join(", ")
                ),
                node_id: Some((*nid).to_string()),
                edge_id: None,
            });
        }
    }
    // File-level node not in any layer.
    for node in &graph.nodes {
        if is_file_level(node.node_type) && !node_to_layers.contains_key(node.id.as_str()) {
            warnings.push(ValidationIssue {
                severity: Severity::Warn,
                code: codes::FILE_NODE_NOT_IN_LAYER.to_string(),
                message: format!(
                    "file-level node '{}' (type {}) is not assigned to any layer",
                    node.id,
                    node.node_type.as_str()
                ),
                node_id: Some(node.id.clone()),
                edge_id: None,
            });
        }
    }

    // ---- tour -------------------------------------------------------------
    for step in &graph.tour {
        for nid in &step.node_ids {
            if !node_ids.contains(nid.as_str()) {
                issues.push(ValidationIssue {
                    severity: Severity::Error,
                    code: codes::TOUR_MISSING_NODE.to_string(),
                    message: format!("tour step {} references unknown node '{}'", step.order, nid),
                    node_id: Some(nid.clone()),
                    edge_id: None,
                });
                errors.push(ValidationError::TourUnknownNode {
                    step: step.order,
                    node: nid.clone(),
                });
            }
        }
    }

    // ---- orphan warning ---------------------------------------------------
    let mut with_edges: HashSet<&str> = HashSet::new();
    for edge in &graph.edges {
        with_edges.insert(edge.source.as_str());
        with_edges.insert(edge.target.as_str());
    }
    for node in &graph.nodes {
        if !with_edges.contains(node.id.as_str()) {
            warnings.push(ValidationIssue {
                severity: Severity::Warn,
                code: codes::ORPHAN_NODE.to_string(),
                message: format!("node '{}' has no incoming or outgoing edges", node.id),
                node_id: Some(node.id.clone()),
                edge_id: None,
            });
        }
    }

    // ---- stats ------------------------------------------------------------
    let mut node_types: BTreeMap<String, usize> = BTreeMap::new();
    let mut hist = ComplexityHistogram::default();
    for node in &graph.nodes {
        *node_types
            .entry(node.node_type.as_str().to_string())
            .or_default() += 1;
        match node.complexity {
            Complexity::Simple => hist.simple += 1,
            Complexity::Moderate => hist.moderate += 1,
            Complexity::Complex => hist.complex += 1,
        }
    }
    let mut edge_types: BTreeMap<String, usize> = BTreeMap::new();
    for edge in &graph.edges {
        *edge_types
            .entry(edge_type_str(edge.edge_type).to_string())
            .or_default() += 1;
    }
    let stats = ValidationStats {
        total_nodes: graph.nodes.len(),
        total_edges: graph.edges.len(),
        total_layers: graph.layers.len(),
        total_tour_steps: graph.tour.len(),
        node_types,
        edge_types,
        complexity_histogram: hist,
    };

    let valid = issues.is_empty();
    ValidationReport {
        valid,
        issues,
        warnings,
        stats,
        errors,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edge::{EdgeDirection, EdgeType, GraphEdge};
    use crate::graph::{KnowledgeGraph, Layer, TourStep};
    use crate::meta::ProjectMeta;
    use crate::node::{Complexity, GraphNode, NodeType};

    fn meta() -> ProjectMeta {
        // Empty `ProjectMeta::default()` is fine — validator never reads it.
        ProjectMeta::default()
    }

    fn node(id: &str, t: NodeType) -> GraphNode {
        GraphNode {
            id: id.to_string(),
            node_type: t,
            name: id.to_string(),
            file_path: None,
            line_range: None,
            summary: format!("summary for {id}"),
            tags: vec!["t".to_string()],
            complexity: Complexity::Simple,
            language_notes: None,
            domain_meta: None,
            knowledge_meta: None,
        }
    }

    fn edge(src: &str, dst: &str) -> GraphEdge {
        GraphEdge {
            source: src.to_string(),
            target: dst.to_string(),
            edge_type: EdgeType::Imports,
            direction: EdgeDirection::Forward,
            description: None,
            weight: 0.5,
        }
    }

    fn empty_graph() -> KnowledgeGraph {
        KnowledgeGraph {
            version: "0".into(),
            kind: None,
            project: meta(),
            nodes: vec![],
            edges: vec![],
            layers: vec![],
            tour: vec![],
        }
    }

    #[test]
    fn orphan_node_emits_warning() {
        let mut g = empty_graph();
        g.nodes.push(node("a", NodeType::Function));
        g.nodes.push(node("b", NodeType::Function));
        g.edges.push(edge("a", "b"));
        g.nodes.push(node("c", NodeType::Function)); // orphan
        let r = validate_graph(&g);
        assert!(r.valid, "should be valid; got {:?}", r.issues);
        let warns: Vec<_> = r
            .warnings
            .iter()
            .filter(|w| w.code == codes::ORPHAN_NODE)
            .collect();
        assert_eq!(warns.len(), 1);
        assert_eq!(warns[0].node_id.as_deref(), Some("c"));
    }

    #[test]
    fn node_in_multiple_layers_emits_warning() {
        let mut g = empty_graph();
        g.nodes.push(node("a", NodeType::Function));
        g.nodes.push(node("b", NodeType::Function));
        g.edges.push(edge("a", "b"));
        g.layers.push(Layer {
            id: "L1".into(),
            name: "L1".into(),
            description: "".into(),
            node_ids: vec!["a".into()],
        });
        g.layers.push(Layer {
            id: "L2".into(),
            name: "L2".into(),
            description: "".into(),
            node_ids: vec!["a".into(), "b".into()],
        });
        let r = validate_graph(&g);
        let multi: Vec<_> = r
            .warnings
            .iter()
            .filter(|w| w.code == codes::NODE_IN_MULTIPLE_LAYERS)
            .collect();
        assert_eq!(multi.len(), 1);
        assert_eq!(multi[0].node_id.as_deref(), Some("a"));
    }

    #[test]
    fn file_node_not_in_layer_emits_warning() {
        let mut g = empty_graph();
        g.nodes.push(node("f1", NodeType::File));
        g.nodes.push(node("f2", NodeType::File));
        g.edges.push(edge("f1", "f2")); // not orphans
        g.layers.push(Layer {
            id: "L1".into(),
            name: "L1".into(),
            description: "".into(),
            node_ids: vec!["f1".into()],
        });
        let r = validate_graph(&g);
        let w: Vec<_> = r
            .warnings
            .iter()
            .filter(|w| w.code == codes::FILE_NODE_NOT_IN_LAYER)
            .collect();
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].node_id.as_deref(), Some("f2"));
    }

    #[test]
    fn missing_required_field_emits_warning() {
        let mut g = empty_graph();
        let mut n1 = node("a", NodeType::Function);
        n1.tags.clear();
        n1.summary = String::new();
        g.nodes.push(n1);
        g.nodes.push(node("b", NodeType::Function));
        g.edges.push(edge("a", "b"));

        let r = validate_graph(&g);
        assert!(r
            .warnings
            .iter()
            .any(|w| w.code == codes::MISSING_TAGS && w.node_id.as_deref() == Some("a")));
        assert!(r
            .warnings
            .iter()
            .any(|w| w.code == codes::MISSING_SUMMARY && w.node_id.as_deref() == Some("a")));
    }

    #[test]
    fn stats_count_correctly() {
        let mut g = empty_graph();
        g.nodes.push(node("a", NodeType::Function));
        let mut b = node("b", NodeType::File);
        b.complexity = Complexity::Moderate;
        g.nodes.push(b);
        let mut c = node("c", NodeType::Class);
        c.complexity = Complexity::Complex;
        g.nodes.push(c);
        g.edges.push(edge("a", "b"));
        let mut e2 = edge("b", "c");
        e2.edge_type = EdgeType::Calls;
        g.edges.push(e2);
        g.layers.push(Layer {
            id: "L".into(),
            name: "L".into(),
            description: "".into(),
            node_ids: vec!["a".into(), "b".into(), "c".into()],
        });
        g.tour.push(TourStep {
            order: 1,
            title: "t".into(),
            description: "d".into(),
            node_ids: vec!["a".into()],
            language_lesson: None,
        });

        let r = validate_graph(&g);
        assert_eq!(r.stats.total_nodes, 3);
        assert_eq!(r.stats.total_edges, 2);
        assert_eq!(r.stats.total_layers, 1);
        assert_eq!(r.stats.total_tour_steps, 1);
        assert_eq!(r.stats.node_types.get("function"), Some(&1));
        assert_eq!(r.stats.node_types.get("file"), Some(&1));
        assert_eq!(r.stats.node_types.get("class"), Some(&1));
        assert_eq!(r.stats.edge_types.get("imports"), Some(&1));
        assert_eq!(r.stats.edge_types.get("calls"), Some(&1));
        assert_eq!(r.stats.complexity_histogram.simple, 1);
        assert_eq!(r.stats.complexity_histogram.moderate, 1);
        assert_eq!(r.stats.complexity_histogram.complex, 1);
    }

    #[test]
    fn missing_node_reference_emits_error() {
        let mut g = empty_graph();
        g.nodes.push(node("a", NodeType::Function));
        g.edges.push(edge("a", "ghost"));
        let r = validate_graph(&g);
        assert!(!r.valid);
        assert!(r
            .issues
            .iter()
            .any(|i| i.code == codes::MISSING_TARGET_NODE));
    }

    #[test]
    fn json_output_round_trips_via_serde_json() {
        let mut g = empty_graph();
        g.nodes.push(node("a", NodeType::Function));
        g.nodes.push(node("b", NodeType::Function));
        g.edges.push(edge("a", "b"));
        let r = validate_graph(&g);
        let json = serde_json::to_string(&r).expect("serialize");
        let v: serde_json::Value = serde_json::from_str(&json).expect("parse");
        // Confirm the public TS shape.
        assert!(v.get("valid").is_some());
        assert!(v.get("issues").unwrap().is_array());
        assert!(v.get("warnings").unwrap().is_array());
        let stats = v.get("stats").expect("stats");
        for k in [
            "totalNodes",
            "totalEdges",
            "totalLayers",
            "totalTourSteps",
            "nodeTypes",
            "edgeTypes",
            "complexityHistogram",
        ] {
            assert!(stats.get(k).is_some(), "stats missing key {k}: {stats}");
        }
        let hist = stats.get("complexityHistogram").unwrap();
        for k in ["simple", "moderate", "complex"] {
            assert!(hist.get(k).is_some(), "histogram missing {k}");
        }
    }
}
