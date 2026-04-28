//! Graph normalization — port of `analyzer/normalize-graph.ts`.
//!
//! Handles malformed LLM outputs: bad ID prefixes, double-prefixed IDs,
//! numeric or aliased complexity values, edge endpoints that need
//! rewriting, and dangling references.
//!
//! ## Behaviour notes
//!
//! - `normalize_complexity` collapses to one canonical match arm. The
//!   previous implementation had a nested `match` whose inner arms were
//!   unreachable (the outer match already returns), and an `unreachable!`
//!   path that wasn't actually unreachable for unknown casing. The
//!   replacement maps every alias directly and falls back to `"moderate"`
//!   for anything we don't recognise — including non-finite numerics and
//!   numerics below `1.0`.

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value};

const VALID_PREFIXES: &[&str] = &[
    "file", "function", "func", // legacy short form — accepted, normalized to "function"
    "class", "module", "concept", "config", "document", "service", "table", "endpoint", "pipeline",
    "schema", "resource", "domain", "flow", "step", "article", "entity", "topic", "claim",
    "source",
];

fn type_to_prefix(node_type: &str) -> &'static str {
    match node_type {
        "file" => "file",
        "function" => "function",
        "class" => "class",
        "module" => "module",
        "concept" => "concept",
        "config" => "config",
        "document" => "document",
        "service" => "service",
        "table" => "table",
        "endpoint" => "endpoint",
        "pipeline" => "pipeline",
        "schema" => "schema",
        "resource" => "resource",
        "domain" => "domain",
        "flow" => "flow",
        "step" => "step",
        "article" => "article",
        "entity" => "entity",
        "topic" => "topic",
        "claim" => "claim",
        "source" => "source",
        _ => "file",
    }
}

fn canonical_prefix(prefix: &str) -> &str {
    // Map legacy short form to the canonical type prefix.
    if prefix == "func" {
        "function"
    } else {
        prefix
    }
}

fn strip_to_valid_prefix(id: &str) -> (Option<&str>, &str) {
    let mut remaining = id;
    loop {
        let Some(colon_idx) = remaining.find(':') else {
            return (None, remaining);
        };
        if colon_idx == 0 {
            return (None, remaining);
        }
        let segment = &remaining[..colon_idx];
        if VALID_PREFIXES.contains(&segment) {
            let rest = &remaining[colon_idx + 1..];
            // Peel double-valid-prefix (e.g. "file:file:path"): use the inner.
            if let Some(inner_idx) = rest.find(':') {
                let inner_seg = &rest[..inner_idx];
                if VALID_PREFIXES.contains(&inner_seg) {
                    remaining = rest;
                    continue;
                }
            }
            return (Some(segment), rest);
        }
        remaining = &remaining[colon_idx + 1..];
    }
}

/// Context fed alongside an ID so we can rebuild it when no valid prefix
/// is present (e.g. a bare `path/to/file.ts:fnName`).
#[derive(Debug, Clone, Default)]
pub struct NormalizeContext<'a> {
    pub node_type: &'a str,
    pub file_path: Option<&'a str>,
    pub name: Option<&'a str>,
    pub parent_flow_slug: Option<&'a str>,
}

/// Normalises a node id to canonical `type:path` form. Idempotent.
pub fn normalize_node_id(id: &str, ctx: NormalizeContext<'_>) -> String {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let expected_prefix = type_to_prefix(ctx.node_type);
    let (prefix, path) = strip_to_valid_prefix(trimmed);

    if let Some(prefix) = prefix {
        let canonical = canonical_prefix(prefix);
        if ctx.node_type == "step" {
            if let Some(file_path) = ctx.file_path {
                let segs: Vec<&str> = path.split(':').collect();
                let step_slug = segs.last().copied().unwrap_or(path);
                let flow_slug = if segs.len() > 1 {
                    segs[segs.len() - 2]
                } else {
                    ""
                };
                if !flow_slug.is_empty() {
                    return format!("{canonical}:{flow_slug}:{file_path}:{step_slug}");
                }
                return format!("{canonical}:{file_path}:{step_slug}");
            }
        }
        return format!("{canonical}:{path}");
    }

    // No valid prefix — bare path. Reconstruct from context.
    if matches!(ctx.node_type, "function" | "class") {
        if let (Some(fp), Some(name)) = (ctx.file_path, ctx.name) {
            return format!("{expected_prefix}:{fp}:{name}");
        }
    }
    if ctx.node_type == "step" {
        if let Some(fp) = ctx.file_path {
            let slug: String = path
                .to_lowercase()
                .chars()
                .map(|c| if c.is_whitespace() { '-' } else { c })
                .collect();
            if let Some(flow_slug) = ctx.parent_flow_slug {
                return format!("{expected_prefix}:{flow_slug}:{fp}:{slug}");
            }
            return format!("{expected_prefix}:{fp}:{slug}");
        }
    }
    format!("{expected_prefix}:{path}")
}

/// Collapses LLM-aliased and numeric complexity values to one of the
/// three canonical strings (`simple` / `moderate` / `complex`). Falls
/// back to `moderate` for unknown inputs.
///
/// Accepted aliases (case-insensitive, trimmed):
/// - `simple` — `simple`, `low`, `easy`, `trivial`, `basic`
/// - `moderate` — `moderate`, `medium`, `intermediate`, `mid`, `average`
/// - `complex` — `complex`, `high`, `hard`, `difficult`, `advanced`
///
/// Numeric inputs use the same buckets as the original TS port: `1..=3`
/// → simple, `4..=6` → moderate, anything finite `> 6` → complex. NaN,
/// infinite, or numerics `< 1.0` fall back to `moderate`.
pub fn normalize_complexity(value: &Value) -> &'static str {
    if let Some(s) = value.as_str() {
        // Single match — the previous nested-match form had an
        // unreachable inner branch that linted as dead code.
        return match s.trim().to_lowercase().as_str() {
            "simple" | "low" | "easy" | "trivial" | "basic" => "simple",
            "moderate" | "medium" | "intermediate" | "mid" | "average" => "moderate",
            "complex" | "high" | "hard" | "difficult" | "advanced" => "complex",
            _ => "moderate",
        };
    }
    if let Some(n) = value.as_f64() {
        if n.is_finite() && n >= 1.0 {
            if n <= 3.0 {
                return "simple";
            }
            if n <= 6.0 {
                return "moderate";
            }
            return "complex";
        }
    }
    "moderate"
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropReason {
    MissingSource,
    MissingTarget,
    MissingBoth,
}

#[derive(Debug, Clone)]
pub struct DroppedEdge {
    pub source: String,
    pub target: String,
    pub edge_type: String,
    pub reason: DropReason,
}

#[derive(Debug, Default, Clone)]
pub struct NormalizationStats {
    pub ids_fixed: u32,
    pub complexity_fixed: u32,
    pub edges_rewritten: u32,
    pub dangling_edges_dropped: u32,
    pub dropped_edges: Vec<DroppedEdge>,
}

pub type RawNode = Map<String, Value>;
pub type RawEdge = Map<String, Value>;

#[derive(Debug, Clone)]
pub struct NormalizeBatchResult {
    pub nodes: Vec<RawNode>,
    pub edges: Vec<RawEdge>,
    pub id_map: HashMap<String, String>,
    pub stats: NormalizationStats,
}

fn infer_type_from_id(id: &str) -> &'static str {
    if let Some(colon_idx) = id.find(':') {
        let prefix = &id[..colon_idx];
        let canonical = canonical_prefix(prefix);
        match canonical {
            "file" => return "file",
            "function" => return "function",
            "class" => return "class",
            "module" => return "module",
            "concept" => return "concept",
            "config" => return "config",
            "document" => return "document",
            "service" => return "service",
            "table" => return "table",
            "endpoint" => return "endpoint",
            "pipeline" => return "pipeline",
            "schema" => return "schema",
            "resource" => return "resource",
            "domain" => return "domain",
            "flow" => return "flow",
            "step" => return "step",
            "article" => return "article",
            "entity" => return "entity",
            "topic" => return "topic",
            "claim" => return "claim",
            "source" => return "source",
            _ => {}
        }
    }
    "file"
}

/// Fixes node ids, normalises complexity, rewrites edge endpoints, and
/// drops dangling edges. Mirrors `normalizeBatchOutput`.
pub fn normalize_batch_output(
    in_nodes: Vec<RawNode>,
    in_edges: Vec<RawEdge>,
) -> NormalizeBatchResult {
    let mut stats = NormalizationStats::default();
    let mut id_map: HashMap<String, String> = HashMap::new();

    // Pre-pass: build flow→slug + step→flow_slug maps (used to reconstruct
    // step ids that need a flow discriminator).
    let mut flow_node_names: HashMap<String, String> = HashMap::new();
    for raw in &in_nodes {
        if raw.get("type").and_then(|v| v.as_str()) == Some("flow") {
            if let (Some(id), Some(name)) = (
                raw.get("id").and_then(|v| v.as_str()),
                raw.get("name").and_then(|v| v.as_str()),
            ) {
                let slug: String = name
                    .to_lowercase()
                    .chars()
                    .map(|c| if c.is_whitespace() { '-' } else { c })
                    .collect();
                flow_node_names.insert(id.to_string(), slug);
            }
        }
    }
    let mut step_to_flow_slug: HashMap<String, String> = HashMap::new();
    for raw in &in_edges {
        if raw.get("type").and_then(|v| v.as_str()) == Some("flow_step") {
            if let (Some(src), Some(tgt)) = (
                raw.get("source").and_then(|v| v.as_str()),
                raw.get("target").and_then(|v| v.as_str()),
            ) {
                if let Some(slug) = flow_node_names.get(src) {
                    step_to_flow_slug.insert(tgt.to_string(), slug.clone());
                }
            }
        }
    }

    // Pass 1: normalise node ids + complexity.
    let mut nodes: Vec<RawNode> = Vec::with_capacity(in_nodes.len());
    for mut raw in in_nodes {
        let old_id = raw
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let node_type = raw
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("file")
            .to_string();
        let file_path = raw
            .get("filePath")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let name = raw
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let parent_flow_slug = if node_type == "step" {
            step_to_flow_slug.get(&old_id).cloned()
        } else {
            None
        };

        let new_id = normalize_node_id(
            &old_id,
            NormalizeContext {
                node_type: &node_type,
                file_path: file_path.as_deref(),
                name: name.as_deref(),
                parent_flow_slug: parent_flow_slug.as_deref(),
            },
        );
        if new_id != old_id {
            stats.ids_fixed += 1;
        }
        id_map.insert(old_id, new_id.clone());
        raw.insert("id".to_string(), Value::String(new_id));

        if let Some(c) = raw.get("complexity").cloned() {
            let normalized = normalize_complexity(&c);
            let already = c.as_str() == Some(normalized);
            if !already {
                stats.complexity_fixed += 1;
                raw.insert(
                    "complexity".to_string(),
                    Value::String(normalized.to_string()),
                );
            }
        }
        nodes.push(raw);
    }

    // Deduplicate nodes (keep last occurrence — newer LLM emissions win).
    let mut last_idx: HashMap<String, usize> = HashMap::new();
    for (i, n) in nodes.iter().enumerate() {
        if let Some(id) = n.get("id").and_then(|v| v.as_str()) {
            last_idx.insert(id.to_string(), i);
        }
    }
    let nodes: Vec<RawNode> = nodes
        .into_iter()
        .enumerate()
        .filter(|(i, n)| {
            n.get("id")
                .and_then(|v| v.as_str())
                .map(|id| last_idx.get(id) == Some(i))
                .unwrap_or(false)
        })
        .map(|(_, n)| n)
        .collect();

    let valid_ids: HashSet<String> = nodes
        .iter()
        .filter_map(|n| n.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();

    // Pass 2: rewrite edge endpoints and dedup.
    let mut edges: Vec<RawEdge> = Vec::with_capacity(in_edges.len());
    let mut seen_edges: HashSet<String> = HashSet::new();
    for mut raw in in_edges {
        let old_source = raw
            .get("source")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let old_target = raw
            .get("target")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let mut new_source = id_map
            .get(&old_source)
            .cloned()
            .unwrap_or_else(|| old_source.clone());
        let mut new_target = id_map
            .get(&old_target)
            .cloned()
            .unwrap_or_else(|| old_target.clone());

        if !valid_ids.contains(&new_source) {
            let inferred = infer_type_from_id(&new_source);
            let normalized = normalize_node_id(
                &new_source,
                NormalizeContext {
                    node_type: inferred,
                    ..Default::default()
                },
            );
            if valid_ids.contains(&normalized) {
                new_source = normalized;
            }
        }
        if !valid_ids.contains(&new_target) {
            let inferred = infer_type_from_id(&new_target);
            let normalized = normalize_node_id(
                &new_target,
                NormalizeContext {
                    node_type: inferred,
                    ..Default::default()
                },
            );
            if valid_ids.contains(&normalized) {
                new_target = normalized;
            }
        }

        if new_source != old_source || new_target != old_target {
            stats.edges_rewritten += 1;
        }

        if !valid_ids.contains(&new_source) || !valid_ids.contains(&new_target) {
            let missing_source = !valid_ids.contains(&new_source);
            let missing_target = !valid_ids.contains(&new_target);
            stats.dangling_edges_dropped += 1;
            stats.dropped_edges.push(DroppedEdge {
                source: new_source,
                target: new_target,
                edge_type: raw
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                reason: match (missing_source, missing_target) {
                    (true, true) => DropReason::MissingBoth,
                    (true, false) => DropReason::MissingSource,
                    (false, true) => DropReason::MissingTarget,
                    (false, false) => unreachable!(),
                },
            });
            continue;
        }

        let edge_type = raw
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let key = format!("{new_source}|{new_target}|{edge_type}");
        if !seen_edges.insert(key) {
            continue;
        }
        raw.insert("source".to_string(), Value::String(new_source));
        raw.insert("target".to_string(), Value::String(new_target));
        edges.push(raw);
    }

    NormalizeBatchResult {
        nodes,
        edges,
        id_map,
        stats,
    }
}
