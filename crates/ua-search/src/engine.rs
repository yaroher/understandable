//! `nucleo-matcher`-backed fuzzy ranker.

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Matcher, Utf32String};
use ua_core::{GraphNode, NodeType};

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub node_id: String,
    /// Normalised in `[0, 1]` — `0` is best, matching Fuse.js semantics.
    pub score: f32,
}

#[derive(Debug, Clone, Default)]
pub struct SearchOptions {
    pub types: Vec<NodeType>,
    pub limit: Option<usize>,
}

/// Pre-converted UTF-32 haystacks for one [`GraphNode`]. `Utf32String`
/// construction is the dominant cost of `search()` — by caching it we
/// avoid 4 conversions × N nodes on every query.
struct Indexed {
    node_id: String,
    node_type: NodeType,
    name: Utf32String,
    tags: Utf32String,
    summary: Utf32String,
    lang_notes: Utf32String,
}

impl Indexed {
    fn from_node(node: &GraphNode) -> Self {
        let tags_joined = node.tags.join(" ");
        let lang_notes_owned = node.language_notes.clone().unwrap_or_default();
        Self {
            node_id: node.id.clone(),
            node_type: node.node_type,
            name: Utf32String::from(node.name.as_str()),
            tags: Utf32String::from(tags_joined.as_str()),
            summary: Utf32String::from(node.summary.as_str()),
            lang_notes: Utf32String::from(lang_notes_owned.as_str()),
        }
    }
}

pub struct SearchEngine {
    nodes: Vec<GraphNode>,
    /// Parallel cache: `indexed[i]` corresponds to `nodes[i]`. Built
    /// once on construction / `replace` so search calls re-use the
    /// same allocations across queries.
    indexed: Vec<Indexed>,
}

impl SearchEngine {
    pub fn new(nodes: Vec<GraphNode>) -> Self {
        let indexed = nodes.iter().map(Indexed::from_node).collect();
        Self { nodes, indexed }
    }

    pub fn nodes(&self) -> &[GraphNode] {
        &self.nodes
    }

    pub fn replace(&mut self, nodes: Vec<GraphNode>) {
        self.indexed = nodes.iter().map(Indexed::from_node).collect();
        self.nodes = nodes;
    }

    /// Returns up to `limit` matches, sorted best-first.
    pub fn search(&self, query: &str, opts: &SearchOptions) -> Vec<SearchResult> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Vec::new();
        }
        let limit = opts.limit.unwrap_or(50);
        let allowed: Option<Vec<NodeType>> = if opts.types.is_empty() {
            None
        } else {
            Some(opts.types.clone())
        };

        // Build per-token patterns. Each token contributes its best per-field
        // score; a node's overall score is the *max* across tokens (OR
        // semantics, matching the TS port's `term1 | term2` extended search).
        let tokens: Vec<Pattern> = trimmed
            .split_whitespace()
            .map(|t| {
                Pattern::new(
                    t,
                    CaseMatching::Ignore,
                    Normalization::Smart,
                    AtomKind::Fuzzy,
                )
            })
            .collect();
        if tokens.is_empty() {
            return Vec::new();
        }

        let mut matcher = Matcher::default();
        let mut scored: Vec<SearchResult> = Vec::new();

        for indexed in &self.indexed {
            if let Some(allowed) = &allowed {
                if !allowed.contains(&indexed.node_type) {
                    continue;
                }
            }

            let mut best_norm = 0.0f32;
            for token in &tokens {
                let raw_name = token
                    .score(indexed.name.slice(..), &mut matcher)
                    .unwrap_or(0);
                let raw_tags = token
                    .score(indexed.tags.slice(..), &mut matcher)
                    .unwrap_or(0);
                let raw_summary = token
                    .score(indexed.summary.slice(..), &mut matcher)
                    .unwrap_or(0);
                let raw_lang = token
                    .score(indexed.lang_notes.slice(..), &mut matcher)
                    .unwrap_or(0);
                let weighted = 0.4 * normalise(raw_name)
                    + 0.3 * normalise(raw_tags)
                    + 0.2 * normalise(raw_summary)
                    + 0.1 * normalise(raw_lang);
                if weighted > best_norm {
                    best_norm = weighted;
                }
            }

            // Map [0, 1]-best-high score to a Fuse-style 0=best, 1=worst.
            let fuse_score = 1.0 - best_norm;
            // Threshold mirroring Fuse's default `threshold: 0.4`.
            if fuse_score >= 0.95 {
                continue;
            }
            scored.push(SearchResult {
                node_id: indexed.node_id.clone(),
                score: fuse_score,
            });
        }

        scored.sort_by(|a, b| {
            a.score
                .partial_cmp(&b.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(limit);
        scored
    }
}

/// Map nucleo's raw integer score to a `[0, 1]` value where `1` is a
/// perfect match. Nucleo's score is unbounded above; we squash with a
/// soft `score / (score + k)` curve so a strong fuzzy hit lands close
/// to 1 without saturating on pathological inputs.
fn normalise(raw: u32) -> f32 {
    if raw == 0 {
        return 0.0;
    }
    let s = raw as f32;
    s / (s + 64.0)
}
