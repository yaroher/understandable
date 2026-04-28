//! `understandable search` — three rankers in one command:
//!
//!   * Default: per-token LIKE prefilter from the storage layer.
//!   * `--fuzzy`: nucleo rerank over the persisted node set.
//!   * `--semantic`: cosine similarity over **persisted** embeddings
//!     produced by `understandable embed`. The query is embedded fresh
//!     each call; the corpus stays warm in the DB.

use std::path::Path;

use clap::{Args as ClapArgs, ValueEnum};
use ua_core::GraphNode;
use ua_llm::{EmbeddingProvider, OpenAiEmbeddings, OLLAMA_EMBED_DEFAULT};
use ua_persist::{ProjectLayout, Storage};
use ua_core::ProjectSettings;
use ua_search::{SearchEngine, SearchOptions};

use crate::commands::embed::{resolve_model_name_from_resolved, ResolvedEmbed};

#[derive(ClapArgs, Debug)]
pub struct Args {
    pub query: String,
    /// Limit the number of results.
    #[arg(long, default_value_t = 25)]
    pub limit: usize,
    /// Run the nucleo fuzzy ranker on top of the LIKE prefilter.
    #[arg(long, conflicts_with = "semantic")]
    pub fuzzy: bool,
    /// Re-rank candidates by embedding cosine similarity.
    #[arg(long)]
    pub semantic: bool,
    /// Embedding backend. Falls back to
    /// `embeddings.provider` in `understandable.yaml` (default `openai`).
    #[arg(long, value_enum)]
    pub embed_provider: Option<EmbedProvider>,
    /// Override the embeddings model.
    #[arg(long)]
    pub embed_model: Option<String>,
    /// Override the embeddings endpoint base URL (openai-compat only).
    #[arg(long)]
    pub embed_endpoint: Option<String>,
    /// Number of LIKE-prefilter candidates fed into the reranker.
    #[arg(long, default_value_t = 200)]
    pub candidate_pool: usize,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum EmbedProvider {
    Openai,
    Ollama,
    Local,
}

pub async fn run(args: Args, project_path: &Path) -> anyhow::Result<()> {
    let layout = ProjectLayout::for_project(project_path);
    let storage = Storage::open(&layout).await?;

    if args.semantic {
        return run_semantic(args, &storage, project_path).await;
    }

    let pool = args.candidate_pool.max(args.limit);
    let ids = storage.search_nodes(&args.query, pool).await?;
    if ids.is_empty() {
        println!("no matches");
        return Ok(());
    }

    if args.fuzzy {
        let graph = storage.load_graph().await?;
        let pool: Vec<GraphNode> = graph
            .nodes
            .into_iter()
            .filter(|n| ids.iter().any(|id| id == &n.id))
            .collect();
        let engine = SearchEngine::new(pool);
        let hits = engine.search(
            &args.query,
            &SearchOptions {
                limit: Some(args.limit),
                ..Default::default()
            },
        );
        for h in hits {
            println!("{:.3}\t{}", h.score, h.node_id);
        }
        return Ok(());
    }

    for id in ids.into_iter().take(args.limit) {
        println!("{id}");
    }
    Ok(())
}

async fn run_semantic(
    args: Args,
    storage: &Storage,
    project_path: &Path,
) -> anyhow::Result<()> {
    // Same resolution logic as `embed` so the persisted rows can be
    // looked up under the same `model` string. Read settings from the
    // user-supplied `project_path` so `--path /elsewhere search
    // --semantic foo` honours the right `understandable.yaml` (the old
    // implementation read `std::env::current_dir()` and silently
    // resolved against the wrong project, breaking semantic search
    // whenever the binary was launched from outside the repo).
    let settings = ProjectSettings::load_or_default(project_path)?;
    let embed_args = crate::commands::embed::Args {
        embed_provider: args.embed_provider,
        embed_model: args.embed_model.clone(),
        embed_endpoint: args.embed_endpoint.clone(),
        reset: false,
        force: false,
        batch_size: None,
    };
    let resolved: ResolvedEmbed = crate::commands::embed::resolve(&embed_args, &settings);
    let model = resolve_model_name_from_resolved(&resolved);

    if storage.embedding_count(&model).await? == 0 {
        anyhow::bail!(
            "no embeddings for model `{model}` — run `understandable embed` first"
        );
    }

    let provider = build_provider_from(&resolved)?;
    let query_vec = provider
        .embed(&[args.query.as_str()])
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("provider returned no vector for the query"))?;

    let hits = storage
        .vector_top_k(&model, &query_vec, args.limit)
        .await?;
    if hits.is_empty() {
        println!("no matches");
        return Ok(());
    }
    for h in hits {
        // Cosine *similarity* = 1 - distance. Print similarity so higher
        // is better, matching the fuzzy ranker's intuition.
        let similarity = 1.0 - h.distance;
        println!("{similarity:.3}\t{}", h.node_id);
    }
    Ok(())
}

/// Build an embedding provider from a fully-resolved settings block.
///
/// Takes a `&ResolvedEmbed` rather than the previous loose tuple so the
/// `batch_size` field actually reaches `LocalEmbeddings::with_batch_size`
/// — the old signature dropped it on the floor and every local embed
/// run defaulted to fastembed's hard-coded batch of 16 regardless of
/// what `embeddings.batch_size` said in the YAML.
pub fn build_provider_from(
    resolved: &ResolvedEmbed,
) -> anyhow::Result<Box<dyn EmbeddingProvider>> {
    match resolved.provider {
        EmbedProvider::Openai => {
            let mut client = OpenAiEmbeddings::new(None)?;
            if let Some(base) = resolved.endpoint.clone() {
                client = client.with_base_url(base);
            }
            if let Some(m) = resolved.model.clone() {
                client = client.with_model(m);
            }
            Ok(Box::new(client))
        }
        EmbedProvider::Ollama => {
            let model = resolved
                .model
                .clone()
                .unwrap_or_else(|| OLLAMA_EMBED_DEFAULT.to_string());
            let mut client = OpenAiEmbeddings::ollama(model);
            if let Some(base) = resolved.endpoint.clone() {
                client = client.with_base_url(base);
            }
            Ok(Box::new(client))
        }
        EmbedProvider::Local => build_local_provider(resolved.model.as_deref(), resolved.batch_size),
    }
}

#[cfg(feature = "local-embeddings")]
fn build_local_provider(
    model: Option<&str>,
    batch_size: usize,
) -> anyhow::Result<Box<dyn EmbeddingProvider>> {
    use ua_llm::LocalEmbeddings;
    let provider = match model {
        Some(name) => {
            let model = parse_local_model(name)?;
            LocalEmbeddings::with_model(model)?
        }
        None => LocalEmbeddings::new()?,
    };
    Ok(Box::new(provider.with_batch_size(batch_size)))
}

#[cfg(not(feature = "local-embeddings"))]
fn build_local_provider(
    _model: Option<&str>,
    _batch_size: usize,
) -> anyhow::Result<Box<dyn EmbeddingProvider>> {
    anyhow::bail!(
        "local embeddings unavailable — recompile understandable with `--features local-embeddings`"
    )
}

#[cfg(feature = "local-embeddings")]
fn parse_local_model(name: &str) -> anyhow::Result<ua_llm::LocalEmbeddingModel> {
    use ua_llm::LocalEmbeddingModel::*;
    Ok(match name {
        "bge-small-en-v1.5" | "bge-small" => BGESmallENV15,
        "bge-base-en-v1.5"  | "bge-base"  => BGEBaseENV15,
        "bge-large-en-v1.5" | "bge-large" => BGELargeENV15,
        "all-MiniLM-L6-v2"  | "minilm"    => AllMiniLML6V2,
        other => anyhow::bail!(
            "unknown local embedding model `{other}`; supported: bge-small-en-v1.5, bge-base-en-v1.5, bge-large-en-v1.5, all-MiniLM-L6-v2"
        ),
    })
}
