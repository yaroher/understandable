//! Graph construction — port of `analyzer/graph-builder.ts`.
//!
//! Accumulates file/function/class/non-code nodes and structural edges
//! (`contains`, `imports`, `calls`) into a [`KnowledgeGraph`]. LLM-supplied
//! summaries and tags arrive via [`FileMeta`] / [`FileWithAnalysisMeta`].
//!
//! ## Behaviour notes
//!
//! - `push_edge` dedupes via a `HashSet<EdgeKey>` whose `Hash`
//!   implementation borrows `&str` slices straight from the incoming
//!   `GraphEdge`. The previous implementation built a fresh
//!   `format!("{:?}|{}|{}", ...)` allocation on *every* call to keep a
//!   hash key alive — that's a String allocation per edge across every
//!   `add_*` call, observable on large graphs. The new path constructs
//!   one `EdgeKey` per *new* edge (only on insert, when we have to own
//!   the strings to store them in the set); existing-edge probes pay
//!   only the hash cost.

use std::borrow::Borrow;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::Path;

use ua_core::{
    Complexity, EdgeDirection, EdgeType, GraphEdge, GraphKind, GraphNode, KnowledgeGraph, Layer,
    NodeType, ProjectMeta, StructuralAnalysis, TourStep,
};
use ua_extract::LanguageRegistry;

/// Per-file meta supplied by the analyzer (LLM or heuristic).
#[derive(Debug, Clone)]
pub struct FileMeta {
    pub summary: String,
    pub tags: Vec<String>,
    pub complexity: Complexity,
}

/// Extension of [`FileMeta`] for files that have a `StructuralAnalysis`.
/// `summaries` maps function/class name → short summary.
#[derive(Debug, Clone)]
pub struct FileWithAnalysisMeta {
    pub file_summary: String,
    pub tags: Vec<String>,
    pub complexity: Complexity,
    pub summaries: HashMap<String, String>,
}

/// Meta for non-code files (config, docs, infra). Caller picks the
/// concrete `node_type`.
#[derive(Debug, Clone)]
pub struct NonCodeFileMeta {
    pub summary: String,
    pub tags: Vec<String>,
    pub complexity: Complexity,
    pub node_type: NodeType,
}

/// Non-code meta with sub-definitions (e.g. SQL tables, Docker services).
#[derive(Debug, Clone, Default)]
pub struct NonCodeFileAnalysisMeta {
    pub base: Option<NonCodeFileMeta>,
    pub definitions: Vec<ua_core::DefinitionInfo>,
    pub services: Vec<ua_core::ServiceInfo>,
    pub endpoints: Vec<ua_core::EndpointInfo>,
    pub steps: Vec<ua_core::StepInfo>,
    pub resources: Vec<ua_core::ResourceInfo>,
    pub sections: Vec<ua_core::SectionInfo>,
}

/// Owned dedup key for the edge set: `(EdgeType, source, target)`.
///
/// `EdgeType` is `Copy + Hash + Eq`, so the only owned-string overhead
/// is the source / target. `Hash` and `Eq` go through the
/// [`EdgeKeyView`] trait so a `&dyn EdgeKeyView` probe (constructed
/// with `&str` slices, no allocation) hashes and compares identically
/// to an owned [`EdgeKey`].
#[derive(Clone)]
struct EdgeKey {
    edge_type: EdgeType,
    source: String,
    target: String,
}

/// Probe interface used for both owned [`EdgeKey`] and a borrowed
/// `(EdgeType, &str, &str)` triple. The trait methods give every
/// implementor the same view of the three fields, so `Hash` and `Eq`
/// can be defined once on `dyn EdgeKeyView`.
trait EdgeKeyView {
    fn edge_type(&self) -> EdgeType;
    fn source(&self) -> &str;
    fn target(&self) -> &str;
}

impl EdgeKeyView for EdgeKey {
    fn edge_type(&self) -> EdgeType {
        self.edge_type
    }
    fn source(&self) -> &str {
        &self.source
    }
    fn target(&self) -> &str {
        &self.target
    }
}

/// Borrowed probe: `(EdgeType, &str, &str)` — *no allocation*. The
/// `EdgeKey` Borrow impl below makes this drop-in usable as a
/// `HashSet::contains` argument.
struct EdgeKeyRef<'a> {
    edge_type: EdgeType,
    source: &'a str,
    target: &'a str,
}

impl<'a> EdgeKeyView for EdgeKeyRef<'a> {
    fn edge_type(&self) -> EdgeType {
        self.edge_type
    }
    fn source(&self) -> &str {
        self.source
    }
    fn target(&self) -> &str {
        self.target
    }
}

impl Hash for dyn EdgeKeyView + '_ {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Mirror what derive(Hash) on a tuple does: hash each field in
        // order. Variants of `EdgeType` are hashed via their derived
        // impl (a `mem::discriminant`-equivalent u32 for the C-like
        // enum), then the two slices are hashed as bytes prefixed by
        // length — exactly what `String::hash` does.
        self.edge_type().hash(state);
        self.source().hash(state);
        self.target().hash(state);
    }
}

impl PartialEq for dyn EdgeKeyView + '_ {
    fn eq(&self, other: &Self) -> bool {
        self.edge_type() == other.edge_type()
            && self.source() == other.source()
            && self.target() == other.target()
    }
}
impl Eq for dyn EdgeKeyView + '_ {}

// Borrow-trait bridge: `HashSet::contains(&q)` where
// `q: &dyn EdgeKeyView` works because `EdgeKey: Borrow<dyn EdgeKeyView>`.
impl<'a> Borrow<dyn EdgeKeyView + 'a> for EdgeKey {
    fn borrow(&self) -> &(dyn EdgeKeyView + 'a) {
        self
    }
}

impl Hash for EdgeKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self as &dyn EdgeKeyView).hash(state);
    }
}

impl PartialEq for EdgeKey {
    fn eq(&self, other: &Self) -> bool {
        (self as &dyn EdgeKeyView) == (other as &dyn EdgeKeyView)
    }
}
impl Eq for EdgeKey {}

/// Mapping from definition `kind` strings (parser-reported) to graph
/// node types. Mirrors `KIND_TO_NODE_TYPE` in `graph-builder.ts`.
fn kind_to_node_type(kind: &str) -> NodeType {
    match kind {
        "table" | "view" | "index" => NodeType::Table,
        "message" | "type" | "enum" => NodeType::Schema,
        "resource" | "module" => NodeType::Resource,
        "service" | "deployment" => NodeType::Service,
        "job" | "stage" | "target" => NodeType::Pipeline,
        "route" | "query" | "mutation" => NodeType::Endpoint,
        "variable" | "output" => NodeType::Config,
        _ => NodeType::Concept,
    }
}

pub struct GraphBuilder {
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
    languages: BTreeSet<String>,
    node_ids: HashSet<String>,
    edge_keys: HashSet<EdgeKey>,
    project_name: String,
    git_hash: String,
    language_registry: LanguageRegistry,
}

impl GraphBuilder {
    pub fn new(project_name: impl Into<String>, git_hash: impl Into<String>) -> Self {
        Self::with_registry(project_name, git_hash, LanguageRegistry::default_registry())
    }

    pub fn with_registry(
        project_name: impl Into<String>,
        git_hash: impl Into<String>,
        registry: LanguageRegistry,
    ) -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
            languages: BTreeSet::new(),
            node_ids: HashSet::new(),
            edge_keys: HashSet::new(),
            project_name: project_name.into(),
            git_hash: git_hash.into(),
            language_registry: registry,
        }
    }

    fn detect_language(&mut self, file_path: &str) {
        if let Some(cfg) = self.language_registry.for_path(Path::new(file_path)) {
            self.languages.insert(cfg.id.clone());
        }
    }

    fn basename(file_path: &str) -> &str {
        file_path.rsplit('/').next().unwrap_or(file_path)
    }

    /// Add a plain file node (no structural analysis).
    pub fn add_file(&mut self, file_path: &str, meta: FileMeta) -> String {
        self.detect_language(file_path);
        let id = format!("file:{file_path}");
        if self.node_ids.insert(id.clone()) {
            self.nodes.push(GraphNode {
                id: id.clone(),
                node_type: NodeType::File,
                name: Self::basename(file_path).to_string(),
                file_path: Some(file_path.to_string()),
                line_range: None,
                summary: meta.summary,
                tags: meta.tags,
                complexity: meta.complexity,
                language_notes: None,
                domain_meta: None,
                knowledge_meta: None,
            });
        }
        id
    }

    /// Add a file node together with its function/class children. Emits
    /// `contains` edges from the file to each child.
    pub fn add_file_with_analysis(
        &mut self,
        file_path: &str,
        analysis: &StructuralAnalysis,
        meta: FileWithAnalysisMeta,
    ) {
        self.detect_language(file_path);
        let file_name = Self::basename(file_path).to_string();
        let file_id = format!("file:{file_path}");
        let complexity = meta.complexity;

        if self.node_ids.insert(file_id.clone()) {
            self.nodes.push(GraphNode {
                id: file_id.clone(),
                node_type: NodeType::File,
                name: file_name,
                file_path: Some(file_path.to_string()),
                line_range: None,
                summary: meta.file_summary,
                tags: meta.tags,
                complexity,
                language_notes: None,
                domain_meta: None,
                knowledge_meta: None,
            });
        }

        for f in &analysis.functions {
            let id = format!("function:{file_path}:{}", f.name);
            if self.node_ids.insert(id.clone()) {
                self.nodes.push(GraphNode {
                    id: id.clone(),
                    node_type: NodeType::Function,
                    name: f.name.clone(),
                    file_path: Some(file_path.to_string()),
                    line_range: Some(f.line_range),
                    summary: meta.summaries.get(&f.name).cloned().unwrap_or_default(),
                    tags: Vec::new(),
                    complexity,
                    language_notes: None,
                    domain_meta: None,
                    knowledge_meta: None,
                });
            }
            self.push_edge(GraphEdge {
                source: file_id.clone(),
                target: id,
                edge_type: EdgeType::Contains,
                direction: EdgeDirection::Forward,
                description: None,
                weight: 1.0,
            });
        }

        for c in &analysis.classes {
            let id = format!("class:{file_path}:{}", c.name);
            if self.node_ids.insert(id.clone()) {
                self.nodes.push(GraphNode {
                    id: id.clone(),
                    node_type: NodeType::Class,
                    name: c.name.clone(),
                    file_path: Some(file_path.to_string()),
                    line_range: Some(c.line_range),
                    summary: meta.summaries.get(&c.name).cloned().unwrap_or_default(),
                    tags: Vec::new(),
                    complexity,
                    language_notes: None,
                    domain_meta: None,
                    knowledge_meta: None,
                });
            }
            self.push_edge(GraphEdge {
                source: file_id.clone(),
                target: id,
                edge_type: EdgeType::Contains,
                direction: EdgeDirection::Forward,
                description: None,
                weight: 1.0,
            });
        }
    }

    /// File-level `imports` edge.
    pub fn add_import_edge(&mut self, from_file: &str, to_file: &str) {
        let edge = GraphEdge {
            source: format!("file:{from_file}"),
            target: format!("file:{to_file}"),
            edge_type: EdgeType::Imports,
            direction: EdgeDirection::Forward,
            description: None,
            weight: 0.7,
        };
        self.push_edge(edge);
    }

    /// Function-level `calls` edge.
    pub fn add_call_edge(
        &mut self,
        caller_file: &str,
        caller_func: &str,
        callee_file: &str,
        callee_func: &str,
    ) {
        let edge = GraphEdge {
            source: format!("function:{caller_file}:{caller_func}"),
            target: format!("function:{callee_file}:{callee_func}"),
            edge_type: EdgeType::Calls,
            direction: EdgeDirection::Forward,
            description: None,
            weight: 0.8,
        };
        self.push_edge(edge);
    }

    /// Add a non-code (config, doc, table, …) parent node. Returns its id.
    pub fn add_non_code_file(&mut self, file_path: &str, meta: NonCodeFileMeta) -> String {
        self.detect_language(file_path);
        let prefix = match meta.node_type {
            NodeType::Config => "config",
            NodeType::Document => "document",
            NodeType::Service => "service",
            NodeType::Table => "table",
            NodeType::Endpoint => "endpoint",
            NodeType::Pipeline => "pipeline",
            NodeType::Schema => "schema",
            NodeType::Resource => "resource",
            _ => "file",
        };
        let id = format!("{prefix}:{file_path}");
        if self.node_ids.insert(id.clone()) {
            self.nodes.push(GraphNode {
                id: id.clone(),
                node_type: meta.node_type,
                name: Self::basename(file_path).to_string(),
                file_path: Some(file_path.to_string()),
                line_range: None,
                summary: meta.summary,
                tags: meta.tags,
                complexity: meta.complexity,
                language_notes: None,
                domain_meta: None,
                knowledge_meta: None,
            });
        }
        id
    }

    /// Add a non-code parent + per-definition / per-service children with
    /// `contains` edges back to the parent.
    pub fn add_non_code_file_with_analysis(
        &mut self,
        file_path: &str,
        meta: NonCodeFileAnalysisMeta,
    ) -> Option<String> {
        let base = meta.base?;
        let complexity = base.complexity;
        let parent_id = self.add_non_code_file(file_path, base);

        for d in &meta.definitions {
            let node = GraphNode {
                id: format!("{}:{file_path}:{}", d.kind, d.name),
                node_type: kind_to_node_type(&d.kind),
                name: d.name.clone(),
                file_path: Some(file_path.to_string()),
                line_range: Some(d.line_range),
                summary: format!("{}: {} ({} fields)", d.kind, d.name, d.fields.len()),
                tags: Vec::new(),
                complexity,
                language_notes: None,
                domain_meta: None,
                knowledge_meta: None,
            };
            self.add_child_node(node, &parent_id);
        }
        for s in &meta.services {
            let summary = match &s.image {
                Some(img) => format!("Service {} (image: {})", s.name, img),
                None => format!("Service {}", s.name),
            };
            let node = GraphNode {
                id: format!("service:{file_path}:{}", s.name),
                node_type: NodeType::Service,
                name: s.name.clone(),
                file_path: Some(file_path.to_string()),
                line_range: s.line_range,
                summary,
                tags: Vec::new(),
                complexity,
                language_notes: None,
                domain_meta: None,
                knowledge_meta: None,
            };
            self.add_child_node(node, &parent_id);
        }
        for e in &meta.endpoints {
            let name = match &e.method {
                Some(m) => format!("{} {}", m, e.path),
                None => e.path.clone(),
            };
            let node = GraphNode {
                id: format!("endpoint:{file_path}:{}", e.path),
                node_type: NodeType::Endpoint,
                name: name.clone(),
                file_path: Some(file_path.to_string()),
                line_range: Some(e.line_range),
                summary: format!("Endpoint: {name}"),
                tags: Vec::new(),
                complexity,
                language_notes: None,
                domain_meta: None,
                knowledge_meta: None,
            };
            self.add_child_node(node, &parent_id);
        }
        for s in &meta.steps {
            let node = GraphNode {
                id: format!("step:{file_path}:{}", s.name),
                node_type: NodeType::Pipeline,
                name: s.name.clone(),
                file_path: Some(file_path.to_string()),
                line_range: Some(s.line_range),
                summary: format!("Step: {}", s.name),
                tags: Vec::new(),
                complexity,
                language_notes: None,
                domain_meta: None,
                knowledge_meta: None,
            };
            self.add_child_node(node, &parent_id);
        }
        for r in &meta.resources {
            let node = GraphNode {
                id: format!("resource:{file_path}:{}", r.name),
                node_type: NodeType::Resource,
                name: r.name.clone(),
                file_path: Some(file_path.to_string()),
                line_range: Some(r.line_range),
                summary: format!("Resource: {} ({})", r.name, r.kind),
                tags: Vec::new(),
                complexity,
                language_notes: None,
                domain_meta: None,
                knowledge_meta: None,
            };
            self.add_child_node(node, &parent_id);
        }
        Some(parent_id)
    }

    fn add_child_node(&mut self, node: GraphNode, parent_id: &str) {
        if !self.node_ids.insert(node.id.clone()) {
            tracing::warn!(target: "ua_analyzer::graph_builder", id = %node.id, "duplicate node id; skipping");
            return;
        }
        let id = node.id.clone();
        self.nodes.push(node);
        self.push_edge(GraphEdge {
            source: parent_id.to_string(),
            target: id,
            edge_type: EdgeType::Contains,
            direction: EdgeDirection::Forward,
            description: None,
            weight: 1.0,
        });
    }

    fn push_edge(&mut self, edge: GraphEdge) {
        // Hash-only probe path: build a borrowed `EdgeKeyRef` over the
        // edge's `&str` fields (zero allocation), check `contains` on
        // the dedup set via the `Borrow<dyn EdgeKeyView>` bridge, and
        // only allocate owned strings when actually inserting a new
        // edge. The previous implementation built a fresh
        // `format!("{:?}|{}|{}", ...)` String *every* call — even on
        // duplicate-edge probes — which dominated the dedup cost on
        // large graphs.
        //
        // cargo bench note: when a workload pushes N edges of which K
        // are duplicates, the old path did N format-and-allocate
        // operations (3 String reservations per format!, plus Debug
        // formatting of the EdgeType variant). The new path does ~0
        // allocations on dedup-hits and exactly 2 String clones per
        // unique edge. Expected speedup grows linearly with the
        // duplicate ratio K/N.
        let probe: &dyn EdgeKeyView = &EdgeKeyRef {
            edge_type: edge.edge_type,
            source: &edge.source,
            target: &edge.target,
        };
        if self.edge_keys.contains(probe) {
            return;
        }
        self.edge_keys.insert(EdgeKey {
            edge_type: edge.edge_type,
            source: edge.source.clone(),
            target: edge.target.clone(),
        });
        self.edges.push(edge);
    }

    /// Materialise the accumulated state into a [`KnowledgeGraph`]. The
    /// returned graph has empty `layers` and `tour` — those are produced
    /// by [`crate::detect_layers`] / [`crate::generate_heuristic_tour`].
    pub fn build(self, analyzed_at: impl Into<String>) -> KnowledgeGraph {
        KnowledgeGraph {
            version: env!("CARGO_PKG_VERSION").to_string(),
            kind: Some(GraphKind::Codebase),
            project: ProjectMeta {
                name: self.project_name,
                languages: self.languages.into_iter().collect(),
                frameworks: Vec::new(),
                description: String::new(),
                analyzed_at: analyzed_at.into(),
                git_commit_hash: self.git_hash,
            },
            nodes: self.nodes,
            edges: self.edges,
            layers: Vec::<Layer>::new(),
            tour: Vec::<TourStep>::new(),
        }
    }
}
