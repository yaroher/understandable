//! `understandable merge` — port of the python `merge-batch-graphs.py`,
//! `merge-subdomain-graphs.py`, and `merge-knowledge-graph.py` scripts.
//!
//! Combines multiple intermediate JSON graph files into one assembled
//! `KnowledgeGraph`. Performs:
//!   * node merge keyed on `id` (later input fields take precedence on
//!     non-empty values, tags unioned + deduped)
//!   * edge merge keyed on `(source, target, edge_type)` with optional
//!     dangling-edge drop
//!   * layer union (deduped by `name`)
//!   * tour: keep first non-empty
//!   * project meta: latest `analyzed_at`, most-recent `git_commit_hash`
//!   * canonical id-normalisation pass (lowercase prefix + `/` separators)
//!
//! Output is written atomically (`<out>.tmp` → `fsync` → rename).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::{Args as ClapArgs, ValueEnum};

use ua_core::{
    EdgeType, GraphEdge, GraphNode, KnowledgeGraph, Layer, TourStep, validate_graph,
};

#[derive(Copy, Clone, Debug, ValueEnum, PartialEq, Eq)]
pub enum MergeKind {
    File,
    Subdomain,
    Knowledge,
}

#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Graph kind being merged (file, subdomain, knowledge).
    #[arg(long, value_enum, default_value = "file")]
    pub kind: MergeKind,
    /// Input JSON graphs. Repeatable: `--inputs a.json --inputs b.json`.
    #[arg(long, required = true)]
    pub inputs: Vec<PathBuf>,
    /// Output path. Defaults to `<cwd>/assembled-graph.json`.
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// Drop edges whose source/target node is missing from merged set.
    #[arg(long, default_value_t = true)]
    pub drop_dangling: bool,
    /// Skip emitting the canonical id-normalisation pass.
    #[arg(long, default_value_t = false)]
    pub no_normalize_ids: bool,
}

pub async fn run(args: Args) -> anyhow::Result<()> {
    if args.inputs.is_empty() {
        anyhow::bail!("merge: need at least one --inputs path");
    }

    // ── Load each input ──────────────────────────────────────────────
    let mut graphs: Vec<KnowledgeGraph> = Vec::with_capacity(args.inputs.len());
    for path in &args.inputs {
        let bytes = fs::read(path)
            .with_context(|| format!("merge: failed to read {}", path.display()))?;
        let graph: KnowledgeGraph = serde_json::from_slice(&bytes)
            .with_context(|| format!("merge: failed to parse {} as KnowledgeGraph", path.display()))?;
        graphs.push(graph);
    }

    let n_inputs = graphs.len();
    let MergeOutcome {
        graph,
        dropped_dangling,
        duplicate_edges,
    } = merge_graphs(graphs, args.drop_dangling, !args.no_normalize_ids);

    // ── Validate (non-fatal) ─────────────────────────────────────────
    let report = validate_graph(&graph);
    if !report.is_valid() {
        eprintln!(
            "[merge] warning: merged graph has {} validation issue(s):",
            report.errors.len()
        );
        for err in &report.errors {
            eprintln!("  - {err}");
        }
    }

    // ── Resolve output path ──────────────────────────────────────────
    let out_path = match args.out {
        Some(p) => p,
        None => std::env::current_dir()
            .context("merge: failed to read cwd for default output path")?
            .join("assembled-graph.json"),
    };

    // ── Serialize + atomic write ─────────────────────────────────────
    let body = serde_json::to_string_pretty(&graph)
        .context("merge: failed to serialize merged graph")?;
    atomic_write(&out_path, body.as_bytes())
        .with_context(|| format!("merge: failed to write {}", out_path.display()))?;

    println!(
        "merged {} inputs → {} nodes, {} edges (dropped {} dangling, {} duplicates)",
        n_inputs,
        graph.nodes.len(),
        graph.edges.len(),
        dropped_dangling,
        duplicate_edges,
    );

    Ok(())
}

// ── Internal merge engine ────────────────────────────────────────────

struct MergeOutcome {
    graph: KnowledgeGraph,
    dropped_dangling: usize,
    duplicate_edges: usize,
}

fn merge_graphs(
    graphs: Vec<KnowledgeGraph>,
    drop_dangling: bool,
    normalize_ids: bool,
) -> MergeOutcome {
    // ── Project meta: pick the latest analyzed_at, and the version /
    //    description / name from the input that contributed it. Also
    //    track most recent git_commit_hash. ──────────────────────────
    let mut nodes: BTreeMap<String, GraphNode> = BTreeMap::new();
    let mut all_edges: Vec<GraphEdge> = Vec::new();
    let mut all_layers: Vec<Layer> = Vec::new();
    let mut tour: Vec<TourStep> = Vec::new();
    let mut version: String = String::new();
    let mut kind = None;
    let mut project = ua_core::ProjectMeta::default();
    // Track best-so-far analyzed_at to drive project meta picks.
    let mut best_analyzed_at: String = String::new();
    let mut best_commit_hash: String = String::new();

    for g in graphs {
        // Project meta tracking
        if g.project.analyzed_at >= best_analyzed_at {
            best_analyzed_at = g.project.analyzed_at.clone();
            // Use this graph as the project-meta source if it's strictly
            // later or equal — later inputs win on ties.
            project = g.project.clone();
        }
        if g.project.git_commit_hash >= best_commit_hash {
            best_commit_hash = g.project.git_commit_hash.clone();
        }

        if version.is_empty() {
            version = g.version.clone();
        } else if !g.version.is_empty() {
            // Later non-empty version takes precedence (mirrors field merge).
            version = g.version.clone();
        }
        if kind.is_none() {
            kind = g.kind;
        }

        // Tour: keep first non-empty
        if tour.is_empty() && !g.tour.is_empty() {
            tour = g.tour.clone();
        }

        // Nodes — merge by id with field precedence
        for node in g.nodes {
            match nodes.get_mut(&node.id) {
                Some(existing) if existing.node_type == node.node_type => {
                    merge_node_into(existing, node);
                }
                Some(_existing) => {
                    // Type conflict — keep existing, ignore new (later
                    // file_path/name preserved as-is on existing). We do
                    // not silently overwrite a different type.
                }
                None => {
                    nodes.insert(node.id.clone(), node);
                }
            }
        }

        // Edges — accumulate, dedupe later
        all_edges.extend(g.edges);

        // Layers — union, dedup by name
        all_layers.extend(g.layers);
    }

    // Stamp the merged project meta with the most-recent commit hash,
    // even if analyzed_at picked a different input.
    if !best_commit_hash.is_empty() {
        project.git_commit_hash = best_commit_hash;
    }

    // ── Optional id normalisation ────────────────────────────────────
    let mut id_remap: HashMap<String, String> = HashMap::new();
    if normalize_ids {
        let mut normalised: BTreeMap<String, GraphNode> = BTreeMap::new();
        for (id, mut node) in std::mem::take(&mut nodes) {
            let new_id = normalize_id(&id);
            if new_id != id {
                id_remap.insert(id.clone(), new_id.clone());
                node.id = new_id.clone();
            }
            match normalised.get_mut(&new_id) {
                Some(existing) => {
                    // Collision — prefer the larger summary.
                    if node.summary.len() > existing.summary.len() {
                        *existing = node;
                    }
                }
                None => {
                    normalised.insert(new_id, node);
                }
            }
        }
        nodes = normalised;
    }

    // Rewrite layer node_ids and tour node_ids if we remapped any ids.
    if !id_remap.is_empty() {
        for layer in &mut all_layers {
            for nid in &mut layer.node_ids {
                if let Some(new) = id_remap.get(nid) {
                    *nid = new.clone();
                }
            }
        }
        for step in &mut tour {
            for nid in &mut step.node_ids {
                if let Some(new) = id_remap.get(nid) {
                    *nid = new.clone();
                }
            }
        }
        for edge in &mut all_edges {
            if let Some(new) = id_remap.get(&edge.source) {
                edge.source = new.clone();
            }
            if let Some(new) = id_remap.get(&edge.target) {
                edge.target = new.clone();
            }
        }
    }

    // ── Layer dedup by name ──────────────────────────────────────────
    let mut layers_by_name: BTreeMap<String, Layer> = BTreeMap::new();
    for layer in all_layers {
        layers_by_name
            .entry(layer.name.clone())
            .and_modify(|existing| {
                let mut seen: BTreeSet<String> = existing.node_ids.iter().cloned().collect();
                for nid in &layer.node_ids {
                    if seen.insert(nid.clone()) {
                        existing.node_ids.push(nid.clone());
                    }
                }
                if existing.description.is_empty() && !layer.description.is_empty() {
                    existing.description = layer.description.clone();
                }
            })
            .or_insert(layer);
    }
    let layers: Vec<Layer> = layers_by_name.into_values().collect();

    // ── Edge dedup by (source, target, edge_type) ────────────────────
    let mut edges_by_key: HashMap<(String, String, EdgeType), GraphEdge> = HashMap::new();
    let mut duplicate_edges = 0usize;
    for edge in all_edges {
        let key = (edge.source.clone(), edge.target.clone(), edge.edge_type);
        match edges_by_key.get_mut(&key) {
            Some(existing) => {
                duplicate_edges += 1;
                // Keep the higher-weight edge.
                if edge.weight > existing.weight {
                    *existing = edge;
                }
            }
            None => {
                edges_by_key.insert(key, edge);
            }
        }
    }

    // ── Optional dangling-edge drop ──────────────────────────────────
    let mut dropped_dangling = 0usize;
    let edges: Vec<GraphEdge> = if drop_dangling {
        let node_ids: BTreeSet<&str> = nodes.keys().map(String::as_str).collect();
        edges_by_key
            .into_values()
            .filter(|e| {
                let ok = node_ids.contains(e.source.as_str())
                    && node_ids.contains(e.target.as_str());
                if !ok {
                    dropped_dangling += 1;
                }
                ok
            })
            .collect()
    } else {
        edges_by_key.into_values().collect()
    };

    let merged = KnowledgeGraph {
        version: if version.is_empty() {
            env!("CARGO_PKG_VERSION").to_string()
        } else {
            version
        },
        kind,
        project,
        nodes: nodes.into_values().collect(),
        edges,
        layers,
        tour,
    };

    MergeOutcome {
        graph: merged,
        dropped_dangling,
        duplicate_edges,
    }
}

/// Merge `incoming` into `existing` in-place — later-non-empty fields
/// win, tags are unioned + deduped.
fn merge_node_into(existing: &mut GraphNode, incoming: GraphNode) {
    // Scalars: later non-empty wins.
    if !incoming.name.is_empty() {
        existing.name = incoming.name;
    }
    if !incoming.summary.is_empty() {
        existing.summary = incoming.summary;
    }
    existing.complexity = incoming.complexity;
    if let Some(fp) = incoming.file_path {
        if !fp.is_empty() {
            existing.file_path = Some(fp);
        }
    }
    if let Some(lr) = incoming.line_range {
        existing.line_range = Some(lr);
    }
    if let Some(notes) = incoming.language_notes {
        if !notes.is_empty() {
            existing.language_notes = Some(notes);
        }
    }
    if let Some(dm) = incoming.domain_meta {
        existing.domain_meta = Some(dm);
    }
    if let Some(km) = incoming.knowledge_meta {
        existing.knowledge_meta = Some(km);
    }

    // Tags: union + dedup, preserving original order.
    let mut seen: BTreeSet<String> = existing.tags.iter().cloned().collect();
    for tag in incoming.tags {
        if seen.insert(tag.clone()) {
            existing.tags.push(tag);
        }
    }
}

/// Lowercase the prefix and normalise path separators to `/`. Mirrors
/// the spirit of the python `normalize_node_id` but is intentionally
/// conservative — we only touch obvious shape issues.
fn normalize_id(id: &str) -> String {
    if let Some(idx) = id.find(':') {
        let (prefix, rest) = id.split_at(idx);
        let prefix_lc = prefix.to_ascii_lowercase();
        let rest_normalised = rest.replace('\\', "/");
        format!("{prefix_lc}{rest_normalised}")
    } else {
        // No prefix — only normalise separators.
        id.replace('\\', "/")
    }
}

// ── Atomic write helper (tmp + fsync + rename) ───────────────────────

fn atomic_write(dst: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = dst.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "atomic_write: destination has no parent directory",
        )
    })?;
    if !parent.as_os_str().is_empty() && !parent.exists() {
        fs::create_dir_all(parent)?;
    }
    let tmp = tmp_path(dst);
    {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, dst)?;
    sync_parent_dir(parent);
    Ok(())
}

fn tmp_path(dst: &Path) -> PathBuf {
    let mut name = dst
        .file_name()
        .map(|s| s.to_os_string())
        .unwrap_or_default();
    name.push(".tmp");
    let mut tmp = dst.to_path_buf();
    tmp.set_file_name(name);
    tmp
}

#[cfg(unix)]
fn sync_parent_dir(parent: &Path) {
    if let Ok(dir) = fs::File::open(parent) {
        let _ = dir.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_parent_dir(_parent: &Path) {
    // The rename itself is the durability boundary on Windows.
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ua_core::{
        Complexity, EdgeDirection, EdgeType, GraphKind, NodeType, ProjectMeta,
    };

    fn node(id: &str, ty: NodeType, summary: &str, tags: &[&str]) -> GraphNode {
        GraphNode {
            id: id.to_string(),
            node_type: ty,
            name: id.split(':').last().unwrap_or(id).to_string(),
            file_path: None,
            line_range: None,
            summary: summary.to_string(),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            complexity: Complexity::Simple,
            language_notes: None,
            domain_meta: None,
            knowledge_meta: None,
        }
    }

    fn edge(src: &str, tgt: &str, et: EdgeType, weight: f32) -> GraphEdge {
        GraphEdge {
            source: src.to_string(),
            target: tgt.to_string(),
            edge_type: et,
            direction: EdgeDirection::Forward,
            description: None,
            weight,
        }
    }

    fn graph_of(nodes: Vec<GraphNode>, edges: Vec<GraphEdge>) -> KnowledgeGraph {
        KnowledgeGraph {
            version: "0.1.0".to_string(),
            kind: Some(GraphKind::Codebase),
            project: ProjectMeta {
                name: "t".into(),
                languages: vec![],
                frameworks: vec![],
                description: String::new(),
                analyzed_at: "2025-01-01T00:00:00Z".into(),
                git_commit_hash: "abc".into(),
            },
            nodes,
            edges,
            layers: vec![],
            tour: vec![],
        }
    }

    #[test]
    fn merge_two_disjoint_graphs_produces_union() {
        let g1 = graph_of(
            vec![node("file:a", NodeType::File, "a", &[])],
            vec![],
        );
        let g2 = graph_of(
            vec![node("file:b", NodeType::File, "b", &[])],
            vec![],
        );

        let out = merge_graphs(vec![g1, g2], true, false);
        assert_eq!(out.graph.nodes.len(), 2);
        let ids: BTreeSet<&str> = out.graph.nodes.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains("file:a"));
        assert!(ids.contains("file:b"));
        assert_eq!(out.dropped_dangling, 0);
        assert_eq!(out.duplicate_edges, 0);
    }

    #[test]
    fn duplicate_node_id_picks_later_field() {
        let mut older = node("file:a", NodeType::File, "old summary", &["tag1"]);
        older.name = "old-name".to_string();
        let mut newer = node("file:a", NodeType::File, "fresher summary", &["tag2"]);
        newer.name = "new-name".to_string();

        let g1 = graph_of(vec![older], vec![]);
        let g2 = graph_of(vec![newer], vec![]);

        let out = merge_graphs(vec![g1, g2], true, false);
        assert_eq!(out.graph.nodes.len(), 1);
        let n = &out.graph.nodes[0];
        assert_eq!(n.name, "new-name");
        assert_eq!(n.summary, "fresher summary");
    }

    #[test]
    fn dangling_edge_dropped_when_flag_set() {
        let g1 = graph_of(
            vec![node("file:a", NodeType::File, "a", &[])],
            vec![edge("file:a", "file:missing", EdgeType::Imports, 0.5)],
        );

        // drop_dangling = true → edge gone, counter incremented.
        let dropped = merge_graphs(vec![g1.clone()], true, false);
        assert_eq!(dropped.graph.edges.len(), 0);
        assert_eq!(dropped.dropped_dangling, 1);

        // drop_dangling = false → edge retained.
        let kept = merge_graphs(vec![g1], false, false);
        assert_eq!(kept.graph.edges.len(), 1);
        assert_eq!(kept.dropped_dangling, 0);
    }

    #[test]
    fn tags_merged_and_deduped() {
        let n1 = node("file:a", NodeType::File, "a", &["alpha", "beta"]);
        let n2 = node("file:a", NodeType::File, "a", &["beta", "gamma"]);
        let g1 = graph_of(vec![n1], vec![]);
        let g2 = graph_of(vec![n2], vec![]);

        let out = merge_graphs(vec![g1, g2], true, false);
        assert_eq!(out.graph.nodes.len(), 1);
        let mut tags = out.graph.nodes[0].tags.clone();
        tags.sort();
        assert_eq!(tags, vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn id_normalization_lowercases_prefix_and_normalises_separators() {
        assert_eq!(normalize_id("FILE:src/foo.rs"), "file:src/foo.rs");
        assert_eq!(
            normalize_id("Function:src\\foo.rs:bar"),
            "function:src/foo.rs:bar"
        );
        assert_eq!(normalize_id("no-prefix\\path"), "no-prefix/path");
    }

    #[test]
    fn id_normalisation_collision_prefers_longer_summary() {
        let a = node("FILE:src/a.rs", NodeType::File, "short", &[]);
        let mut b = node("file:src/a.rs", NodeType::File, "this is a far longer summary", &[]);
        b.name = "winner".to_string();

        let g = graph_of(vec![a, b], vec![]);
        let out = merge_graphs(vec![g], true, true);
        assert_eq!(out.graph.nodes.len(), 1);
        assert_eq!(out.graph.nodes[0].summary, "this is a far longer summary");
    }

    #[test]
    fn duplicate_edges_count_dedup_and_higher_weight_wins() {
        let g1 = graph_of(
            vec![
                node("file:a", NodeType::File, "a", &[]),
                node("file:b", NodeType::File, "b", &[]),
            ],
            vec![edge("file:a", "file:b", EdgeType::Imports, 0.3)],
        );
        let g2 = graph_of(
            vec![],
            vec![edge("file:a", "file:b", EdgeType::Imports, 0.9)],
        );
        let out = merge_graphs(vec![g1, g2], true, false);
        assert_eq!(out.graph.edges.len(), 1);
        assert!((out.graph.edges[0].weight - 0.9).abs() < f32::EPSILON);
        assert_eq!(out.duplicate_edges, 1);
    }

    /// Cheap RAII tempdir without pulling in the `tempfile` crate.
    struct ScratchDir {
        path: std::path::PathBuf,
    }

    impl ScratchDir {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0);
            let path = std::env::temp_dir().join(format!(
                "ua-cli-merge-test-{tag}-{}-{nanos}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &std::path::Path {
            &self.path
        }
    }

    impl Drop for ScratchDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn atomic_write_round_trips_bytes() {
        let dir = ScratchDir::new("aw");
        let dst = dir.path().join("out.json");
        atomic_write(&dst, b"{\"hello\":1}").unwrap();
        let body = std::fs::read_to_string(&dst).unwrap();
        assert_eq!(body, "{\"hello\":1}");
        // Tmp file should be cleaned up after rename.
        let tmp = tmp_path(&dst);
        assert!(!tmp.exists());
    }

    #[test]
    fn run_end_to_end_writes_assembled_graph() {
        let dir = ScratchDir::new("e2e");
        let g1 = graph_of(
            vec![node("file:a", NodeType::File, "a", &["alpha"])],
            vec![],
        );
        let g2 = graph_of(
            vec![
                node("file:a", NodeType::File, "a-newer", &["beta"]),
                node("file:b", NodeType::File, "b", &[]),
            ],
            vec![edge("file:a", "file:b", EdgeType::Imports, 0.5)],
        );
        let p1 = dir.path().join("g1.json");
        let p2 = dir.path().join("g2.json");
        std::fs::write(&p1, serde_json::to_string(&g1).unwrap()).unwrap();
        std::fs::write(&p2, serde_json::to_string(&g2).unwrap()).unwrap();

        let out = dir.path().join("assembled.json");
        let args = Args {
            kind: MergeKind::File,
            inputs: vec![p1, p2],
            out: Some(out.clone()),
            drop_dangling: true,
            no_normalize_ids: false,
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(run(args)).expect("merge run failed");

        let body = std::fs::read_to_string(&out).unwrap();
        let merged: KnowledgeGraph = serde_json::from_str(&body).unwrap();
        assert_eq!(merged.nodes.len(), 2);
        assert_eq!(merged.edges.len(), 1);
        let mut summaries: Vec<&str> =
            merged.nodes.iter().map(|n| n.summary.as_str()).collect();
        summaries.sort();
        assert_eq!(summaries, vec!["a-newer", "b"]);
    }
}
