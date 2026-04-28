//! Karpathy-style wiki ingest — port of `article-analyzer` agent's
//! deterministic substrate.
//!
//! Walks a markdown directory, parses YAML frontmatter, extracts the
//! first H1 / title, finds `[[wikilinks]]` and `#tags`, and emits an
//! `article` node per file plus `topic` nodes per category. LLM
//! agents may enrich the result with `entity` / `claim` / `source`
//! nodes.
//!
//! ## Behaviour notes
//!
//! - Removed a dead `let articles_for_backlink = articles.clone();` +
//!   `drop(articles_for_backlink)` pair. The clone wasn't read, so it
//!   was a wasted O(n) deep-copy on every wiki ingest.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use ua_core::{
    EdgeDirection, EdgeType, GraphEdge, GraphKind, GraphNode, KnowledgeGraph, KnowledgeMeta,
    NodeType, ProjectMeta,
};

#[derive(Debug, Clone, Default)]
pub struct ParsedArticle {
    pub path: PathBuf,
    pub slug: String,
    pub title: String,
    pub category: Option<String>,
    pub tags: Vec<String>,
    pub wikilinks: Vec<String>,
    pub content: String,
    pub frontmatter: serde_yaml_ng::Value,
}

/// Walk `wiki_root` for `.md` files (gitignore-aware) and parse each.
pub fn parse_wiki(wiki_root: &Path) -> Vec<ParsedArticle> {
    let mut walker = ignore::WalkBuilder::new(wiki_root);
    walker
        .git_ignore(true)
        .git_global(true)
        .add_custom_ignore_filename(".gitignore")
        .hidden(false);
    let mut out = Vec::new();
    for entry in walker.build().filter_map(Result::ok) {
        if !entry.file_type().map(|f| f.is_file()).unwrap_or(false) {
            continue;
        }
        let path = entry.into_path();
        let lower = path.to_string_lossy().to_lowercase();
        if !lower.ends_with(".md") && !lower.ends_with(".mdx") {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let parsed = parse_markdown(&path, &raw, wiki_root);
        out.push(parsed);
    }
    out
}

fn parse_markdown(path: &Path, raw: &str, wiki_root: &Path) -> ParsedArticle {
    let (frontmatter_yaml, body) = split_frontmatter(raw);
    let frontmatter: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(frontmatter_yaml).unwrap_or(serde_yaml_ng::Value::Null);
    let title = pick_title(&frontmatter, body, path);
    let category = frontmatter
        .get("category")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let tags = frontmatter
        .get("tags")
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let wikilinks = extract_wikilinks(body);
    let slug = path_slug(path, wiki_root);

    ParsedArticle {
        path: path.to_path_buf(),
        slug,
        title,
        category,
        tags,
        wikilinks,
        content: body.to_string(),
        frontmatter,
    }
}

fn split_frontmatter(raw: &str) -> (&str, &str) {
    let trimmed = raw.trim_start_matches('\u{feff}');
    if let Some(rest) = trimmed.strip_prefix("---\n") {
        if let Some(end) = rest.find("\n---\n") {
            let yaml = &rest[..end];
            let body = &rest[end + 5..];
            return (yaml, body);
        }
        if let Some(end) = rest.find("\n---\r\n") {
            let yaml = &rest[..end];
            let body = &rest[end + 6..];
            return (yaml, body);
        }
    }
    ("", trimmed)
}

fn pick_title(frontmatter: &serde_yaml_ng::Value, body: &str, path: &Path) -> String {
    if let Some(t) = frontmatter.get("title").and_then(|v| v.as_str()) {
        return t.to_string();
    }
    for line in body.lines() {
        if let Some(stripped) = line.strip_prefix("# ") {
            return stripped.trim().to_string();
        }
    }
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("untitled")
        .to_string()
}

fn extract_wikilinks(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'[' && bytes[i + 1] == b'[' {
            // Find closing `]]`.
            let start = i + 2;
            let mut j = start;
            while j + 1 < bytes.len() && !(bytes[j] == b']' && bytes[j + 1] == b']') {
                j += 1;
            }
            if j + 1 < bytes.len() {
                let raw = &body[start..j];
                let target = raw.split('|').next().unwrap_or(raw).trim().to_string();
                if !target.is_empty() {
                    out.push(target);
                }
                i = j + 2;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn path_slug(path: &Path, root: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.with_extension("")
        .to_string_lossy()
        .replace('\\', "/")
}

/// Build a `kind=knowledge` graph from parsed articles. Articles
/// referencing each other via `[[wikilinks]]` get `cites` edges; each
/// category becomes a `topic` node linked via `categorized_under`.
pub fn build_knowledge_graph(
    project_name: &str,
    git_hash: &str,
    analyzed_at: &str,
    articles: Vec<ParsedArticle>,
) -> KnowledgeGraph {
    let mut nodes: Vec<GraphNode> = Vec::new();
    let mut edges: Vec<GraphEdge> = Vec::new();
    let mut seen_edge: BTreeSet<String> = BTreeSet::new();
    let mut push_edge = |edges: &mut Vec<GraphEdge>, e: GraphEdge| {
        let key = format!("{}|{}|{:?}", e.source, e.target, e.edge_type);
        if seen_edge.insert(key) {
            edges.push(e);
        }
    };

    let mut topics: BTreeMap<String, String> = BTreeMap::new(); // category → topic_id
    let mut by_slug: BTreeMap<String, String> = BTreeMap::new(); // slug → article_id
    let mut by_title: BTreeMap<String, String> = BTreeMap::new(); // title → article_id

    for art in &articles {
        let id = format!("article:{}", art.slug);
        by_slug.insert(art.slug.clone(), id.clone());
        by_title.insert(art.title.clone(), id.clone());
        nodes.push(GraphNode {
            id,
            node_type: NodeType::Article,
            name: art.title.clone(),
            file_path: Some(art.slug.clone()),
            line_range: None,
            summary: first_paragraph(&art.content),
            tags: art.tags.clone(),
            complexity: ua_core::Complexity::Moderate,
            language_notes: None,
            domain_meta: None,
            knowledge_meta: Some(KnowledgeMeta {
                wikilinks: Some(art.wikilinks.clone()),
                backlinks: None,
                category: art.category.clone(),
                content: Some(art.content.clone()),
            }),
        });

        if let Some(cat) = &art.category {
            let topic_id = topics
                .entry(cat.clone())
                .or_insert_with(|| format!("topic:{}", slugify(cat)))
                .clone();
            push_edge(
                &mut edges,
                GraphEdge {
                    source: format!("article:{}", art.slug),
                    target: topic_id,
                    edge_type: EdgeType::CategorizedUnder,
                    direction: EdgeDirection::Forward,
                    description: None,
                    weight: 1.0,
                },
            );
        }
    }

    // Topic nodes (deduped via BTreeMap).
    for (cat, id) in &topics {
        nodes.push(GraphNode {
            id: id.clone(),
            node_type: NodeType::Topic,
            name: cat.clone(),
            file_path: None,
            line_range: None,
            summary: format!("Category: {cat}"),
            tags: vec!["category".into()],
            complexity: ua_core::Complexity::Simple,
            language_notes: None,
            domain_meta: None,
            knowledge_meta: None,
        });
    }

    // Wikilink edges: article → article (cites).
    let mut backlinks: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for art in &articles {
        let src_id = format!("article:{}", art.slug);
        for link in &art.wikilinks {
            let target = by_slug
                .get(link)
                .cloned()
                .or_else(|| by_title.get(link).cloned());
            if let Some(target_id) = target {
                push_edge(
                    &mut edges,
                    GraphEdge {
                        source: src_id.clone(),
                        target: target_id.clone(),
                        edge_type: EdgeType::Cites,
                        direction: EdgeDirection::Forward,
                        description: Some(format!("wikilink to {link}")),
                        weight: 0.8,
                    },
                );
                backlinks.entry(target_id).or_default().push(src_id.clone());
            }
        }
    }

    // Fold backlinks back into KnowledgeMeta.
    for n in nodes.iter_mut() {
        if let Some(meta) = n.knowledge_meta.as_mut() {
            if let Some(b) = backlinks.remove(&n.id) {
                meta.backlinks = Some(b);
            }
        }
    }

    KnowledgeGraph {
        version: env!("CARGO_PKG_VERSION").to_string(),
        kind: Some(GraphKind::Knowledge),
        project: ProjectMeta {
            name: project_name.to_string(),
            languages: vec!["markdown".into()],
            frameworks: Vec::new(),
            description: format!("Knowledge graph from {} articles", articles.len()),
            analyzed_at: analyzed_at.to_string(),
            git_commit_hash: git_hash.to_string(),
        },
        nodes,
        edges,
        layers: Vec::new(),
        tour: Vec::new(),
    }
}

fn first_paragraph(content: &str) -> String {
    let mut buf = String::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !buf.is_empty() {
                break;
            }
            continue;
        }
        if trimmed.starts_with('#') {
            continue;
        }
        if !buf.is_empty() {
            buf.push(' ');
        }
        buf.push_str(trimmed);
    }
    if buf.len() > 280 {
        buf.truncate(277);
        buf.push_str("...");
    }
    buf
}

fn slugify(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}
