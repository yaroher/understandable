//! `understandable embed` — bulk-embed graph nodes and store the
//! vectors in the persisted DB so `search --semantic` doesn't have to
//! re-embed the corpus on every call.
//!
//! The model is picked via `--embed-provider` (same enum as `search`).
//! Each node's text is `name :: summary :: tags`; the row is skipped
//! when the stored `text_hash` matches, so re-running is cheap.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use clap::Args as ClapArgs;
use ua_core::{GraphNode, ProjectSettings};
use ua_llm::{LOCAL_EMBED_DEFAULT, OLLAMA_EMBED_DEFAULT, OPENAI_EMBED_DEFAULT};
use ua_persist::{ProjectLayout, Storage};

use crate::commands::search::EmbedProvider;

#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Embedding backend. Falls back to `embeddings.provider` in
    /// `understandable.yaml`; defaults to `openai`.
    #[arg(long, value_enum)]
    pub embed_provider: Option<EmbedProvider>,
    /// Override the embeddings model. Falls back to
    /// `embeddings.model` in `understandable.yaml`.
    #[arg(long)]
    pub embed_model: Option<String>,
    /// Override the embeddings endpoint base URL (openai-compat only).
    #[arg(long)]
    pub embed_endpoint: Option<String>,
    /// Drop every existing embedding for the model first. Required
    /// when switching to a model with a different vector dimension.
    #[arg(long)]
    pub reset: bool,
    /// Re-embed every node even if its `text_hash` already matches.
    #[arg(long)]
    pub force: bool,
    /// How many texts per provider call. Falls back to
    /// `embeddings.batch_size` in `understandable.yaml`; default 32.
    #[arg(long)]
    pub batch_size: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct ResolvedEmbed {
    pub provider: EmbedProvider,
    pub model: Option<String>,
    pub endpoint: Option<String>,
    pub batch_size: usize,
}

pub fn resolve(args: &Args, settings: &ProjectSettings) -> ResolvedEmbed {
    let provider = args
        .embed_provider
        .unwrap_or_else(|| parse_provider(&settings.embeddings.provider));
    let model = args
        .embed_model
        .clone()
        .or_else(|| settings.embeddings.model.clone());
    let endpoint = args
        .embed_endpoint
        .clone()
        .or_else(|| settings.embeddings.endpoint.clone());
    let batch_size = args.batch_size.unwrap_or(settings.embeddings.batch_size);
    ResolvedEmbed {
        provider,
        model,
        endpoint,
        batch_size: batch_size.max(1),
    }
}

fn parse_provider(name: &str) -> EmbedProvider {
    match name.to_ascii_lowercase().as_str() {
        "ollama" => EmbedProvider::Ollama,
        "local" => EmbedProvider::Local,
        _ => EmbedProvider::Openai,
    }
}

pub async fn run(args: Args, project_path: &Path) -> anyhow::Result<()> {
    let settings = ProjectSettings::load_or_default(project_path)?;
    let resolved = resolve(&args, &settings);

    let layout = ProjectLayout::for_project(project_path);
    let storage = Storage::open(&layout).await?;
    let graph = storage.load_graph().await?;
    if graph.nodes.is_empty() {
        anyhow::bail!("no graph found — run `understandable analyze` before embedding");
    }

    let model = resolve_model_name_from_resolved(&resolved);
    let provider = crate::commands::search::build_provider_from(&resolved)?;

    if args.reset {
        storage.reset_embeddings(&model).await?;
    }

    // Probe the dimension once via a tiny sample so we can register
    // the (model, dim) pair up front instead of failing on the first
    // upsert.
    let probe = provider.embed(&["dimension probe"]).await?;
    let dim = probe
        .first()
        .map(|v| v.len())
        .ok_or_else(|| anyhow::anyhow!("provider returned no embedding for the probe input"))?;
    storage.ensure_embeddings_table(&model, dim).await?;

    // Pull every existing `text_hash` for this model in one shot.
    // The new in-memory backend exposes a bulk accessor; the previous
    // implementation fired one query per node, which on a 10k-node
    // graph turned an O(n) embed run into O(n) DB calls.
    let mut existing_hashes: HashMap<String, String> = HashMap::new();
    if !args.force {
        existing_hashes = storage.embedding_hashes_for(&model).await?;
    }

    let mut work: Vec<(String, String, String)> = Vec::new(); // (id, text, hash)
    for node in &graph.nodes {
        let text = node_text(node);
        let hash = ua_persist::blake3_string(text.as_bytes());
        if !args.force {
            if let Some(existing) = existing_hashes.get(&node.id) {
                if existing == &hash {
                    continue;
                }
            }
        }
        work.push((node.id.clone(), text, hash));
    }
    if work.is_empty() {
        println!(
            "embeddings up to date for `{model}` ({} nodes already covered)",
            graph.nodes.len()
        );
        return Ok(());
    }

    let total = work.len();
    let mut done = 0usize;
    // Run the per-batch network calls in parallel up to
    // `embeddings.concurrency`; each task only does the I/O. Storage
    // upserts stay on the main task (they hit an async-mutex-protected
    // state — pushing them into spawned tasks would just contend on
    // the same lock). Saturate at 1 so a 0-config doesn't deadlock.
    //
    // TODO: OpenAI embeddings responses include a `usage` block with
    // `prompt_tokens` / `total_tokens`. `OpenAiEmbeddings::embed`
    // currently throws this away and we can't edit `ua-llm` from this
    // crate boundary; once a `embed_with_usage` lands we should fold
    // those numbers into a `TokenTotals`-style summary line here.
    let concurrency = settings.embeddings.concurrency.max(1);
    let provider = Arc::new(provider);
    let sem = Arc::new(tokio::sync::Semaphore::new(concurrency));
    /// Per-batch task output: `(node_id, text_hash)` pairs zipped with
    /// the same number of `Vec<f32>` rows the provider returned. Pulled
    /// out into an alias so clippy stops yelling about the inline type.
    type EmbedTaskOutput = anyhow::Result<(Vec<(String, String)>, Vec<Vec<f32>>)>;
    let mut joinset: tokio::task::JoinSet<EmbedTaskOutput> = tokio::task::JoinSet::new();

    for chunk in work.chunks(resolved.batch_size) {
        let permit = sem
            .clone()
            .acquire_owned()
            .await
            .expect("semaphore not closed");
        let provider = provider.clone();
        // Stash the (node_id, hash) pairs so the upsert step can match
        // returned vectors back up. Texts are owned so the spawned task
        // can borrow without lifetime gymnastics.
        let pairs: Vec<(String, String)> = chunk
            .iter()
            .map(|(id, _, h)| (id.clone(), h.clone()))
            .collect();
        let texts: Vec<String> = chunk.iter().map(|(_, t, _)| t.clone()).collect();
        joinset.spawn(async move {
            let _permit = permit;
            let refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
            let vectors = provider.embed(&refs).await?;
            Ok((pairs, vectors))
        });
    }

    // Drain every batch — failures from any single batch must not
    // discard the in-flight vectors that already arrived from the
    // provider. Successes still upsert; errors are accumulated and
    // surfaced once the joinset is empty so the caller sees "X
    // batch(es) failed; partial progress saved".
    //
    // The previous behaviour was `return Err(e)` on the first sad
    // result, which dropped the joinset and threw away every
    // already-paid-for batch still landing in the receive queue.
    let mut errors: Vec<anyhow::Error> = Vec::new();
    while let Some(joined) = joinset.join_next().await {
        let (pairs, vectors) = match joined {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => {
                errors.push(e);
                continue;
            }
            Err(e) => {
                errors.push(anyhow::anyhow!("embed task panicked: {e}"));
                continue;
            }
        };
        if vectors.len() != pairs.len() {
            errors.push(anyhow::anyhow!(
                "provider returned {} vectors for {} inputs",
                vectors.len(),
                pairs.len()
            ));
            continue;
        }
        let mut batch_failed = false;
        for ((node_id, hash), vec) in pairs.iter().zip(vectors.iter()) {
            if vec.len() != dim {
                errors.push(anyhow::anyhow!(
                    "vector dim drift: got {} expected {dim}",
                    vec.len()
                ));
                batch_failed = true;
                break;
            }
            if let Err(e) = storage
                .upsert_node_embedding(node_id, &model, vec, hash)
                .await
            {
                errors.push(anyhow::anyhow!("upsert failed for node `{node_id}`: {e}"));
                batch_failed = true;
                break;
            }
        }
        if !batch_failed {
            done += pairs.len();
            tracing::info!(done, total, "embedded batch");
        }
    }

    storage.save(&layout).await?;
    if !errors.is_empty() {
        // Log every individual error so a user inspecting `tracing`
        // output can see which batch actually died. The bail message
        // stays short — it's the post-condition the caller wires
        // their own UX around.
        for err in &errors {
            tracing::warn!(error = %err, "embed batch failed");
        }
        anyhow::bail!(
            "embed: {} batch(es) failed; partial progress saved ({done}/{total} node(s) into `{model}`, dim={dim})",
            errors.len()
        );
    }
    println!("embedded {done}/{total} node(s) into `{model}` (dim={dim})");
    Ok(())
}

pub fn node_text(node: &GraphNode) -> String {
    let parts: Vec<&str> = [
        node.name.as_str(),
        node.summary.as_str(),
        // Joined tags string is built lazily below; keep this slot empty
        // here so we don't allocate when tags is empty.
        "",
    ]
    .into_iter()
    .map(str::trim)
    .filter(|s| !s.is_empty())
    .collect();
    let tags = node.tags.join(",");
    let mut combined = parts.join(" :: ");
    let tags_trimmed = tags.trim();
    if !tags_trimmed.is_empty() {
        if !combined.is_empty() {
            combined.push_str(" :: ");
        }
        combined.push_str(tags_trimmed);
    }
    // Last-resort fallback so providers (Ollama in particular) never
    // get an empty string — bge-m3 returns NaN on `""` and crashes the
    // whole batch with a 500.
    if combined.is_empty() {
        combined = format!("node:{}", node.id);
    }
    combined
}

pub fn resolve_model_name_from_resolved(r: &ResolvedEmbed) -> String {
    if let Some(m) = &r.model {
        return m.clone();
    }
    default_model_for(r.provider).to_string()
}

pub fn default_model_for(p: EmbedProvider) -> &'static str {
    match p {
        EmbedProvider::Openai => OPENAI_EMBED_DEFAULT,
        EmbedProvider::Ollama => OLLAMA_EMBED_DEFAULT,
        EmbedProvider::Local => LOCAL_EMBED_DEFAULT,
    }
}
