//! Map a list of changed file paths to graph nodes + ripple effects.

use std::collections::{HashMap, HashSet};

use serde::Serialize;
use ua_core::{Complexity, EdgeType, GraphEdge, GraphNode, KnowledgeGraph, Layer, NodeType};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffContext {
    pub project_name: String,
    /// The raw list of changed files supplied by the caller — kept on the
    /// struct so the formatter can surface unmapped paths even when no
    /// graph node points at them.
    pub changed_files: Vec<String>,
    pub changed_nodes: Vec<GraphNode>,
    pub affected_nodes: Vec<GraphNode>,
    pub impacted_edges: Vec<GraphEdge>,
    pub affected_layers: Vec<Layer>,
    pub unmapped_files: Vec<String>,
}

pub fn build_diff_context(graph: &KnowledgeGraph, changed_files: &[String]) -> DiffContext {
    // O(N) index file_path -> nodes once instead of scanning the whole
    // node list for every changed file. Many graphs have several nodes
    // (file + child symbols) per path so we stash a `Vec<&GraphNode>`.
    let mut nodes_by_path: HashMap<&str, Vec<&GraphNode>> = HashMap::new();
    for node in &graph.nodes {
        if let Some(p) = node.file_path.as_deref() {
            nodes_by_path.entry(p).or_default().push(node);
        }
    }

    let mut changed_ids: HashSet<String> = HashSet::new();
    let mut unmapped_files: Vec<String> = Vec::new();

    for file in changed_files {
        match nodes_by_path.get(file.as_str()) {
            Some(nodes) if !nodes.is_empty() => {
                for n in nodes {
                    changed_ids.insert(n.id.clone());
                }
            }
            _ => unmapped_files.push(file.clone()),
        }
    }

    // Pull `contains` children of changed file nodes into the changed set.
    for e in &graph.edges {
        if e.edge_type == EdgeType::Contains && changed_ids.contains(&e.source) {
            changed_ids.insert(e.target.clone());
        }
    }

    let changed_nodes: Vec<GraphNode> = graph
        .nodes
        .iter()
        .filter(|n| changed_ids.contains(&n.id))
        .cloned()
        .collect();

    // 1-hop neighbours of changed nodes.
    let mut affected_ids: HashSet<String> = HashSet::new();
    let mut impacted_edges: Vec<GraphEdge> = Vec::new();
    for e in &graph.edges {
        let src_changed = changed_ids.contains(&e.source);
        let tgt_changed = changed_ids.contains(&e.target);
        if src_changed || tgt_changed {
            impacted_edges.push(e.clone());
            if src_changed && !changed_ids.contains(&e.target) {
                affected_ids.insert(e.target.clone());
            }
            if tgt_changed && !changed_ids.contains(&e.source) {
                affected_ids.insert(e.source.clone());
            }
        }
    }

    let affected_nodes: Vec<GraphNode> = graph
        .nodes
        .iter()
        .filter(|n| affected_ids.contains(&n.id))
        .cloned()
        .collect();

    // Membership tests over `all_impacted` happened inside an
    // `Iterator::any` linear scan of `node_ids`; converting to a
    // `HashSet` keeps this linear in `node_ids.len()` rather than
    // quadratic in the total impacted-id count.
    let mut all_impacted: HashSet<String> = changed_ids.clone();
    all_impacted.extend(affected_ids.iter().cloned());
    let affected_layers: Vec<Layer> = graph
        .layers
        .iter()
        .filter(|l| l.node_ids.iter().any(|id| all_impacted.contains(id)))
        .cloned()
        .collect();

    DiffContext {
        project_name: graph.project.name.clone(),
        changed_files: changed_files.to_vec(),
        changed_nodes,
        affected_nodes,
        impacted_edges,
        affected_layers,
        unmapped_files,
    }
}

pub fn format_diff_analysis(ctx: &DiffContext) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("# Diff Analysis: {}", ctx.project_name));
    lines.push(String::new());

    lines.push("## Changed Components".into());
    lines.push(String::new());
    if ctx.changed_nodes.is_empty() {
        lines.push("No mapped components found for changed files.".into());
    } else {
        for n in &ctx.changed_nodes {
            lines.push(format!(
                "- **{}** ({}) — {}",
                n.name,
                node_type_label(n.node_type),
                n.summary
            ));
            if let Some(p) = &n.file_path {
                lines.push(format!("  - File: `{p}`"));
            }
            lines.push(format!(
                "  - Complexity: {}",
                complexity_label(n.complexity)
            ));
        }
    }
    lines.push(String::new());

    lines.push("## Affected Components".into());
    lines.push(String::new());
    if ctx.affected_nodes.is_empty() {
        lines.push("No downstream impact detected.".into());
    } else {
        lines.push("These components are connected to changed code and may need attention:".into());
        lines.push(String::new());
        for n in &ctx.affected_nodes {
            lines.push(format!(
                "- **{}** ({}) — {}",
                n.name,
                node_type_label(n.node_type),
                n.summary
            ));
        }
    }
    lines.push(String::new());

    lines.push("## Affected Layers".into());
    lines.push(String::new());
    if ctx.affected_layers.is_empty() {
        lines.push("No layers affected.".into());
    } else {
        for l in &ctx.affected_layers {
            lines.push(format!("- **{}**: {}", l.name, l.description));
        }
    }
    lines.push(String::new());

    if !ctx.impacted_edges.is_empty() {
        lines.push("## Impacted Relationships".into());
        lines.push(String::new());
        for e in &ctx.impacted_edges {
            lines.push(format!(
                "- {} --[{}]--> {}",
                e.source,
                edge_type_label(e.edge_type),
                e.target
            ));
        }
        lines.push(String::new());
    }

    if !ctx.unmapped_files.is_empty() {
        lines.push("## Unmapped Files".into());
        lines.push(String::new());
        lines.push("These changed files are not yet in the knowledge graph:".into());
        lines.push(String::new());
        for f in &ctx.unmapped_files {
            lines.push(format!("- `{f}`"));
        }
        lines.push(String::new());
    }

    lines.push("## Risk Assessment".into());
    lines.push(String::new());
    let complex_changes: Vec<&GraphNode> = ctx
        .changed_nodes
        .iter()
        .filter(|n| n.complexity == Complexity::Complex)
        .collect();
    let cross_layer_count = ctx
        .affected_layers
        .iter()
        .map(|l| l.id.as_str())
        .collect::<std::collections::HashSet<_>>()
        .len();

    let mut wrote_any = false;
    if !complex_changes.is_empty() {
        let names: Vec<&str> = complex_changes.iter().map(|n| n.name.as_str()).collect();
        lines.push(format!(
            "- **High complexity**: {} complex component(s) changed: {}",
            complex_changes.len(),
            names.join(", ")
        ));
        wrote_any = true;
    }
    if cross_layer_count > 1 {
        lines.push(format!(
            "- **Cross-layer impact**: Changes span {cross_layer_count} architectural layers"
        ));
        wrote_any = true;
    }
    if ctx.affected_nodes.len() > 5 {
        lines.push(format!(
            "- **Wide blast radius**: {} components affected downstream",
            ctx.affected_nodes.len()
        ));
        wrote_any = true;
    }
    if !ctx.unmapped_files.is_empty() {
        lines.push(format!(
            "- **New/unmapped files**: {} files not in the knowledge graph (may need re-analysis)",
            ctx.unmapped_files.len()
        ));
        wrote_any = true;
    }
    if !wrote_any {
        lines.push("- **Low risk**: Changes are localized with limited downstream impact.".into());
    }
    lines.push(String::new());
    lines.join("\n")
}

fn node_type_label(t: NodeType) -> &'static str {
    t.as_str()
}

fn complexity_label(c: Complexity) -> &'static str {
    match c {
        Complexity::Simple => "simple",
        Complexity::Moderate => "moderate",
        Complexity::Complex => "complex",
    }
}

fn edge_type_label(t: EdgeType) -> &'static str {
    use EdgeType::*;
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

/// Pull staged + unstaged paths from `git status --porcelain=v2 -z`.
///
/// `--porcelain=v2 -z` is the only stable way to read paths that
/// contain spaces, quotes, or newlines: every record is NUL-terminated
/// instead of newline-terminated, and v2 spells out rename pairs as
/// two NUL-separated paths rather than the ambiguous `old -> new` arrow
/// notation that v1 used (and which broke whenever a path itself
/// contained ` -> `).
///
/// Record format per `git-status(1)`:
///
/// ```text
/// 1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>\0
/// 2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <X><score> <path>\0<orig-path>\0
/// u <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>\0
/// ? <path>\0
/// ! <path>\0
/// ```
pub fn changed_files_from_git(project: &std::path::Path) -> Vec<String> {
    let Ok(out) = std::process::Command::new("git")
        .args(["status", "--porcelain=v2", "-z"])
        .current_dir(project)
        .output()
    else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    parse_porcelain_v2(&stdout)
}

/// Split a porcelain v2 `-z` payload into the *new* paths.
///
/// Exposed at crate-public visibility so the unit tests can feed in
/// canned byte streams without spinning up a real git repo.
pub(crate) fn parse_porcelain_v2(payload: &str) -> Vec<String> {
    let mut files: Vec<String> = Vec::new();
    let mut iter = payload.split('\0').peekable();

    while let Some(record) = iter.next() {
        if record.is_empty() {
            continue;
        }
        let mut parts = record.splitn(2, ' ');
        let kind = parts.next().unwrap_or("");
        let rest = parts.next().unwrap_or("");
        match kind {
            // Ordinary entry: `<XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>`
            // — 7 metadata fields after the leading kind, then the
            // path (which may itself contain spaces).
            "1" => {
                if let Some(path) = field_after(rest, 7) {
                    files.push(path.to_string());
                }
            }
            // Rename / copy: the record holds the *new* path after 8
            // metadata fields; the *original* path follows as the
            // next NUL-terminated chunk and is currently uninteresting
            // for our purposes.
            "2" => {
                if let Some(path) = field_after(rest, 8) {
                    files.push(path.to_string());
                }
                // Drain the original-path field so the outer loop
                // doesn't try to parse it as a fresh record.
                let _ = iter.next();
            }
            // Unmerged: `<XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>`
            // — 9 metadata fields then path.
            "u" => {
                if let Some(path) = field_after(rest, 9) {
                    files.push(path.to_string());
                }
            }
            // Untracked / ignored: `? <path>` or `! <path>` — the
            // entire remainder is the path verbatim.
            "?" | "!" if !rest.is_empty() => {
                files.push(rest.to_string());
            }
            // Header line ("# branch.oid ...") or anything we don't
            // recognise: skip.
            _ => {}
        }
    }

    files
}

/// Skip `n_fields` whitespace-separated fields at the start of `s` and
/// return the remainder verbatim (which is the path — paths may
/// contain embedded spaces in porcelain v2 because records are
/// NUL-terminated, not whitespace-terminated). Returns `None` when
/// `s` doesn't have enough fields.
fn field_after(s: &str, n_fields: usize) -> Option<&str> {
    let mut rest = s;
    for _ in 0..n_fields {
        let (_field, tail) = rest.split_once(' ')?;
        rest = tail;
    }
    if rest.is_empty() {
        None
    } else {
        Some(rest)
    }
}

#[cfg(test)]
mod tests {
    use super::parse_porcelain_v2;

    #[test]
    fn parses_ordinary_change() {
        // `1 .M N... <path>`
        let raw = "1 .M N... 100644 100644 100644 abc def src/lib.rs\0";
        assert_eq!(parse_porcelain_v2(raw), vec!["src/lib.rs".to_string()]);
    }

    #[test]
    fn parses_path_with_space() {
        // The path itself contains a space — porcelain v2 keeps it
        // intact because records are NUL-terminated.
        let raw = "1 .M N... 100644 100644 100644 abc def some dir/with space.rs\0";
        assert_eq!(
            parse_porcelain_v2(raw),
            vec!["some dir/with space.rs".to_string()]
        );
    }

    #[test]
    fn parses_rename_with_arrow_in_path() {
        // Rename: new path is `weird -> name.rs` (literal arrow in the
        // path), original is `old.rs`. Porcelain v1 would have been
        // unparseable here; v2 separates new and old with a NUL.
        let raw = "2 R. N... 100644 100644 100644 abc def R100 weird -> name.rs\0old.rs\0";
        assert_eq!(
            parse_porcelain_v2(raw),
            vec!["weird -> name.rs".to_string()]
        );
    }

    #[test]
    fn parses_untracked() {
        let raw = "? new_file.rs\0";
        assert_eq!(parse_porcelain_v2(raw), vec!["new_file.rs".to_string()]);
    }

    #[test]
    fn ignores_header_lines() {
        let raw = "# branch.oid abc123\0# branch.head main\0? real.rs\0";
        assert_eq!(parse_porcelain_v2(raw), vec!["real.rs".to_string()]);
    }
}
