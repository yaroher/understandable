//! `understandable knowledge <wiki>` — Karpathy-style markdown ingest.
//!
//! By default this runs only the deterministic substrate
//! ([`parse_wiki`] → [`build_knowledge_graph`]). Pass `--with-llm` to
//! follow up with the article-analyzer LLM pass: each article body is
//! sent to Anthropic alongside a list of every other article id/title
//! in the graph; the model returns implicit edges
//! (`cites`/`claims`/`contradicts`/`builds_on`/`refutes`) which we
//! merge into the graph before persisting.
//!
//! ## Edge-type fallbacks
//!
//! [`ua_core::EdgeType`] does not currently have variants for every
//! relation the prompt asks the model to surface. We map as follows:
//!
//! | LLM label     | EdgeType variant         | Notes                                   |
//! | ------------- | ------------------------ | --------------------------------------- |
//! | `cites`       | [`EdgeType::Cites`]      | exact match                             |
//! | `contradicts` | [`EdgeType::Contradicts`]| exact match                             |
//! | `builds_on`   | [`EdgeType::BuildsOn`]   | exact match                             |
//! | `claims`      | [`EdgeType::Exemplifies`]| no `Claim` edge in core; closest fit    |
//! | `refutes`     | [`EdgeType::Contradicts`]| no `Refutes` variant; same direction    |
//!
//! Unknown labels are dropped with a `tracing::warn!`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::Args as ClapArgs;
use serde::Deserialize;
use ua_analyzer::{build_knowledge_graph, parse_wiki};
use ua_core::{EdgeDirection, EdgeType, GraphEdge, KnowledgeGraph, NodeType, ProjectSettings};
use ua_llm::{AnthropicClient, CompleteRequest};
use ua_persist::{blake3_string, ProjectLayout, Storage};

use crate::commands::usage::TokenTotals;
use crate::util::time::iso8601_now;

#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Path to the wiki / Karpathy-style knowledge base.
    pub wiki: PathBuf,
    /// Enrich the knowledge graph with article-analyzer LLM passes.
    /// Implicit edges (claim/cites/contradicts/builds-on) are inferred
    /// from the article bodies. Off by default.
    #[arg(long)]
    pub with_llm: bool,
    /// LLM concurrency for article enrichment. Falls back to
    /// `llm.concurrency` in `understandable.yaml`.
    #[arg(long)]
    pub llm_concurrency: Option<usize>,
    /// Cap on articles sent to the LLM in one run.
    #[arg(long, default_value_t = 50)]
    pub llm_max_articles: usize,
    /// Anthropic model override.
    #[arg(long)]
    pub llm_model: Option<String>,
}

pub async fn run(args: Args, project: &Path) -> anyhow::Result<()> {
    let articles = parse_wiki(&args.wiki);
    if articles.is_empty() {
        anyhow::bail!("no markdown files found under {}", args.wiki.display());
    }
    let project_name = project
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("project")
        .to_string();
    let git_hash = ua_persist::staleness::current_git_head(project).unwrap_or_default();
    let analyzed_at = iso8601_now();
    let mut graph = build_knowledge_graph(&project_name, &git_hash, &analyzed_at, articles);

    let layout = ProjectLayout::for_project(project);
    let storage = Arc::new(Storage::open_kind(&layout, "knowledge").await?);

    if args.with_llm {
        // Hard-fail on a missing key would be hostile to muscle-memory
        // users who add `--with-llm` without an env var present. Warn
        // and fall back to the deterministic graph instead.
        if std::env::var("ANTHROPIC_API_KEY").is_err() {
            tracing::warn!(
                target: "ua_cli::knowledge",
                "ANTHROPIC_API_KEY missing — skipping --with-llm enrichment"
            );
        } else {
            let settings = ProjectSettings::load_or_default(project)?;
            let concurrency = args
                .llm_concurrency
                .unwrap_or(settings.llm.concurrency)
                .max(1);
            run_llm_enrichment(
                &mut graph,
                args.llm_model.clone(),
                args.llm_max_articles,
                concurrency,
                storage.clone(),
            )
            .await?;
        }
    }

    storage.save_graph(&graph).await?;
    storage.save_kind(&layout, "knowledge").await?;
    println!(
        "knowledge graph: {} nodes ({} articles, {} topics), {} edges",
        graph.nodes.len(),
        count_kind(&graph, NodeType::Article),
        count_kind(&graph, NodeType::Topic),
        graph.edges.len(),
    );
    Ok(())
}

fn count_kind(g: &KnowledgeGraph, k: NodeType) -> usize {
    g.nodes.iter().filter(|n| n.node_type == k).count()
}

/// Article-analyzer enrichment pass.
///
/// For every `article` node (capped at `cap`), build a prompt
/// containing the article body plus the id/title of every *other*
/// article in the graph; ask Anthropic for implicit
/// claim/cites/contradicts/builds-on edges; merge the parsed edges
/// into `graph`.
///
/// Concurrency: one [`tokio::task::JoinSet`] gated by a
/// [`tokio::sync::Semaphore`] of size `concurrency`. The cache short-
/// circuit checks `Storage::llm_output_for(article_id, prompt_hash,
/// body_hash)` before any API call. Every successful call rolls its
/// `TokenUsage` into a [`TokenTotals`] which prints one summary line
/// at the end of the run.
async fn run_llm_enrichment(
    graph: &mut KnowledgeGraph,
    model: Option<String>,
    cap: usize,
    concurrency: usize,
    storage: Arc<Storage>,
) -> anyhow::Result<()> {
    let client = Arc::new(AnthropicClient::new(None)?);

    // Build the catalog of "other articles" once — it's identical for
    // every prompt except for the one self-row we drop. Cheap enough
    // (id + title strings) that the per-article filter doesn't matter.
    let articles: Vec<(String, String, String)> = graph
        .nodes
        .iter()
        .filter(|n| n.node_type == NodeType::Article)
        .map(|n| {
            let body = n
                .knowledge_meta
                .as_ref()
                .and_then(|m| m.content.clone())
                .unwrap_or_default();
            (n.id.clone(), n.name.clone(), body)
        })
        .collect();

    if articles.is_empty() {
        return Ok(());
    }

    let catalog = Arc::new(build_catalog(&articles));
    let known_ids: std::collections::HashSet<String> =
        graph.nodes.iter().map(|n| n.id.clone()).collect();

    let sem = Arc::new(tokio::sync::Semaphore::new(concurrency));
    let mut joinset: tokio::task::JoinSet<anyhow::Result<Option<ArticleOutcome>>> =
        tokio::task::JoinSet::new();

    let total_articles = articles.len();
    for (article_id, title, body) in articles.into_iter().take(cap) {
        let permit = sem.clone().acquire_owned().await.expect("semaphore not closed");
        let storage = storage.clone();
        let client = client.clone();
        let model = model.clone();
        let catalog = catalog.clone();
        joinset.spawn(async move {
            let _permit = permit;
            process_one_article(&article_id, &title, &body, &catalog, &client, model.as_deref(), &storage).await
        });
    }

    let mut totals = TokenTotals::default();
    let mut hits = 0usize;
    let mut misses = 0usize;
    let mut new_edges: Vec<GraphEdge> = Vec::new();

    while let Some(joined) = joinset.join_next().await {
        match joined {
            Ok(Ok(Some(outcome))) => {
                if outcome.cached {
                    hits += 1;
                } else {
                    misses += 1;
                }
                if let Some(u) = outcome.usage.as_ref() {
                    totals.add(u);
                }
                for edge in outcome.edges {
                    if known_ids.contains(&edge.target) && edge.target != edge.source {
                        new_edges.push(edge);
                    }
                }
            }
            Ok(Ok(None)) => {}
            Ok(Err(e)) => tracing::warn!(error = %e, "article enrichment task failed"),
            Err(e) => tracing::warn!(error = %e, "article enrichment task panicked"),
        }
    }

    if total_articles > cap {
        tracing::info!(
            "LLM article cap ({cap}) reached; {} articles skipped",
            total_articles - cap
        );
    }

    // De-dup against existing edges so an LLM "cites X" doesn't
    // duplicate the deterministic wikilink edge.
    let existing: std::collections::HashSet<String> = graph
        .edges
        .iter()
        .map(|e| format!("{}|{}|{:?}", e.source, e.target, e.edge_type))
        .collect();
    let mut appended = 0usize;
    for edge in new_edges {
        let key = format!("{}|{}|{:?}", edge.source, edge.target, edge.edge_type);
        if !existing.contains(&key) {
            graph.edges.push(edge);
            appended += 1;
        }
    }

    let total = hits + misses;
    let model_name = model
        .as_deref()
        .unwrap_or(ua_llm::ANTHROPIC_DEFAULT)
        .to_string();
    if totals.is_zero() {
        if total > 0 {
            println!(
                "llm-enriched {total} articles (cached: {hits}/{total}, +{appended} edges)"
            );
        }
    } else {
        let usd = totals.estimate_usd(&model_name);
        println!(
            "llm-enriched {total} articles (cache hits: {hits}/{total}, input={} output={} cache_read={} tokens, ≈${:.4}, +{appended} edges)",
            totals.input, totals.output, totals.cache_read, usd
        );
    }
    Ok(())
}

/// Build the "you may cite any of these" appendix. Truncated at
/// ~12k chars so the largest catalogs stay well under the 100k-token
/// envelope the prompt budget targets (the article body itself can
/// add another ~30k chars before we hit the cap).
fn build_catalog(articles: &[(String, String, String)]) -> String {
    let mut out = String::with_capacity(articles.len() * 64);
    for (id, title, _) in articles {
        out.push_str("- ");
        out.push_str(id);
        out.push_str(" — ");
        out.push_str(title);
        out.push('\n');
        if out.len() > 12_000 {
            out.push_str("... (catalog truncated)\n");
            break;
        }
    }
    out
}

/// Outcome of one per-article LLM task.
struct ArticleOutcome {
    edges: Vec<GraphEdge>,
    usage: Option<ua_llm::TokenUsage>,
    cached: bool,
}

#[derive(Debug, Deserialize)]
struct LlmEdgesReply {
    #[serde(default)]
    edges: Vec<LlmEdge>,
}

#[derive(Debug, Deserialize)]
struct LlmEdge {
    target: String,
    edge_type: String,
    #[serde(default = "default_weight")]
    weight: f32,
}

fn default_weight() -> f32 {
    0.5
}

/// Per-article worker. Mirrors the cache → call → parse pattern from
/// `analyze::process_one_file`. Returns `Ok(None)` when the article
/// has no body and `Ok(Some(_))` for both cache hits and live calls.
async fn process_one_article(
    article_id: &str,
    title: &str,
    body: &str,
    catalog: &str,
    client: &AnthropicClient,
    model: Option<&str>,
    storage: &Storage,
) -> anyhow::Result<Option<ArticleOutcome>> {
    if body.trim().is_empty() {
        return Ok(None);
    }

    let (system, user) = article_analyzer_prompts(article_id, title, body, catalog);
    let body_hash = blake3_string(body.as_bytes());
    let prompt_hash = blake3_string(format!("{system}|{user}").as_bytes());

    if let Some(cached) = storage
        .llm_output_for(article_id, &prompt_hash, &body_hash)
        .await?
    {
        match parse_edges(&cached, article_id) {
            Ok(edges) => {
                return Ok(Some(ArticleOutcome {
                    edges,
                    usage: None,
                    cached: true,
                }));
            }
            Err(e) => {
                tracing::warn!(
                    article = %article_id,
                    error = %e,
                    "cached llm reply failed to parse — re-running"
                );
            }
        }
    }

    let mut chat = CompleteRequest::user(user)
        .with_system(system)
        .with_max_tokens(2048)
        .with_temperature(0.2)
        // Same system prompt for every article — caching cuts the
        // per-call input bill to 10% after the first hit.
        .with_system_cache();
    if let Some(m) = model {
        chat = chat.with_model(m.to_string());
    }

    let result = match client.complete_with_usage(chat).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(article = %article_id, error = %e, "llm call failed");
            return Ok(None);
        }
    };

    let edges = match parse_edges(&result.text, article_id) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(
                article = %article_id,
                error = %e,
                "could not parse llm reply"
            );
            return Ok(None);
        }
    };

    if let Err(e) = storage
        .cache_llm_output(article_id, &prompt_hash, &body_hash, &result.text)
        .await
    {
        tracing::warn!(error = %e, article = %article_id, "could not cache llm output");
    }

    Ok(Some(ArticleOutcome {
        edges,
        usage: Some(result.usage),
        cached: false,
    }))
}

/// `(system, user)` pair for the article-analyzer pass. The system
/// prompt is identical across articles so the
/// [`CompleteRequest::with_system_cache`] block actually amortises.
fn article_analyzer_prompts(
    article_id: &str,
    title: &str,
    body: &str,
    catalog: &str,
) -> (String, String) {
    let system = "You are an article analyser. Given an article, identify which other \
articles it claims, cites, contradicts, or builds on. Return JSON: \
{ \"edges\": [{\"target\": \"<node_id>\", \"edge_type\": \"cites|claims|contradicts|builds_on|refutes\", \"weight\": 0..1}] }. \
Reply with JSON only — no prose, no markdown fences. Targets MUST be \
node ids drawn from the supplied catalog. Skip self-references. Return \
an empty edges array when no implicit links are present."
        .to_string();
    let body_clipped = trim_for_prompt(body, 24_000);
    let user = format!(
        "Article id: {article_id}\nTitle: {title}\n\nBody:\n{body_clipped}\n\nCatalog of other articles (id — title):\n{catalog}\nReturn the JSON.",
    );
    (system, user)
}

fn trim_for_prompt(content: &str, max_chars: usize) -> String {
    if content.chars().count() <= max_chars {
        return content.to_string();
    }
    let mut out: String = content.chars().take(max_chars).collect();
    out.push_str("\n... (truncated)");
    out
}

/// Tolerantly parse the LLM reply: strip code fences, deserialise the
/// `{ edges: [...] }` envelope, then map each edge label to an
/// [`EdgeType`] using the fallback table at the top of this module.
/// Edges whose `target` equals `source` (self-references) are dropped
/// — caller filters unknown ids separately so it can also weed out
/// references to nodes that vanished between runs.
fn parse_edges(raw: &str, source_id: &str) -> Result<Vec<GraphEdge>, serde_json::Error> {
    let cleaned = strip_code_fence(raw);
    let parsed: LlmEdgesReply = serde_json::from_str(cleaned)?;
    let mut out = Vec::with_capacity(parsed.edges.len());
    for edge in parsed.edges {
        let Some(edge_type) = map_edge_type(&edge.edge_type) else {
            tracing::warn!(
                source = %source_id,
                label = %edge.edge_type,
                "unknown edge_type from llm — dropping"
            );
            continue;
        };
        if edge.target == source_id {
            continue;
        }
        out.push(GraphEdge {
            source: source_id.to_string(),
            target: edge.target,
            edge_type,
            direction: EdgeDirection::Forward,
            description: Some(format!("llm: {}", edge.edge_type)),
            weight: edge.weight.clamp(0.0, 1.0),
        });
    }
    Ok(out)
}

/// Apply the fallback table from the module docs.
fn map_edge_type(label: &str) -> Option<EdgeType> {
    match label.trim().to_lowercase().as_str() {
        "cites" => Some(EdgeType::Cites),
        "contradicts" => Some(EdgeType::Contradicts),
        "builds_on" | "builds-on" | "buildson" => Some(EdgeType::BuildsOn),
        // No `Claim` edge in core; `Exemplifies` is the closest fit.
        "claims" | "claim" => Some(EdgeType::Exemplifies),
        // No `Refutes` variant; folded into `Contradicts`.
        "refutes" => Some(EdgeType::Contradicts),
        _ => None,
    }
}

fn strip_code_fence(s: &str) -> &str {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("```json") {
        return rest.trim_start_matches('\n').trim_end_matches("```").trim();
    }
    if let Some(rest) = s.strip_prefix("```") {
        return rest.trim_start_matches('\n').trim_end_matches("```").trim();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// Wraps `Args` so `clap::Parser` can be derived for the test
    /// without committing the binary's full subcommand tree.
    #[derive(Parser, Debug)]
    struct Harness {
        #[command(flatten)]
        args: Args,
    }

    #[test]
    fn parses_with_llm_and_max_articles() {
        let parsed = Harness::try_parse_from([
            "test",
            "wiki/",
            "--with-llm",
            "--llm-max-articles",
            "5",
        ])
        .expect("parse");
        assert!(parsed.args.with_llm);
        assert_eq!(parsed.args.llm_max_articles, 5);
        assert_eq!(parsed.args.wiki, PathBuf::from("wiki/"));
        assert!(parsed.args.llm_concurrency.is_none());
        assert!(parsed.args.llm_model.is_none());
    }

    #[test]
    fn defaults_disable_llm() {
        let parsed = Harness::try_parse_from(["test", "wiki/"]).expect("parse");
        assert!(!parsed.args.with_llm);
        assert_eq!(parsed.args.llm_max_articles, 50);
    }

    #[test]
    fn map_edge_type_covers_fallbacks() {
        assert_eq!(map_edge_type("cites"), Some(EdgeType::Cites));
        assert_eq!(map_edge_type("contradicts"), Some(EdgeType::Contradicts));
        assert_eq!(map_edge_type("builds_on"), Some(EdgeType::BuildsOn));
        assert_eq!(map_edge_type("builds-on"), Some(EdgeType::BuildsOn));
        // Falls back to Exemplifies — no `Claim` edge in core.
        assert_eq!(map_edge_type("claims"), Some(EdgeType::Exemplifies));
        // Falls back to Contradicts — no `Refutes` variant.
        assert_eq!(map_edge_type("refutes"), Some(EdgeType::Contradicts));
        assert_eq!(map_edge_type("UNKNOWN"), None);
    }

    #[test]
    fn parse_edges_clean_json() {
        let raw = r#"{"edges":[{"target":"article:foo","edge_type":"cites","weight":0.9}]}"#;
        let edges = parse_edges(raw, "article:bar").expect("parse");
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].target, "article:foo");
        assert_eq!(edges[0].edge_type, EdgeType::Cites);
        assert!((edges[0].weight - 0.9).abs() < 1e-6);
    }

    #[test]
    fn parse_edges_drops_self_reference() {
        let raw = r#"{"edges":[{"target":"article:bar","edge_type":"cites"}]}"#;
        let edges = parse_edges(raw, "article:bar").expect("parse");
        assert!(edges.is_empty());
    }

    #[test]
    fn parse_edges_strips_fence() {
        let raw = "```json\n{\"edges\":[{\"target\":\"article:x\",\"edge_type\":\"builds_on\"}]}\n```";
        let edges = parse_edges(raw, "article:y").expect("parse");
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].edge_type, EdgeType::BuildsOn);
    }

    #[test]
    fn parse_edges_drops_unknown_label() {
        let raw = r#"{"edges":[{"target":"article:x","edge_type":"frobnicates"}]}"#;
        let edges = parse_edges(raw, "article:y").expect("parse");
        assert!(edges.is_empty());
    }
}
