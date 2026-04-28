---
title: embed
sidebar_position: 3
---

# `understandable embed`

Bulk-embeds every node in the persisted graph and stores the f32
vectors inside the same `graph.tar.zst`. Once populated, `search
--semantic` ranks nodes by cosine similarity without re-embedding the
corpus on every call.

## Synopsis

```bash
understandable embed [--embed-provider {openai,ollama,local}] \
                     [--embed-model <id>] [--embed-endpoint <url>] \
                     [--reset] [--force] [--batch-size <N>]
```

## Three providers

| Provider | Default model              | Auth             | Notes                                                                                                                          |
|----------|----------------------------|------------------|--------------------------------------------------------------------------------------------------------------------------------|
| `openai` | `text-embedding-3-small`   | `OPENAI_API_KEY` | Default. Hits `api.openai.com`. Pass `--embed-endpoint` to talk to any OpenAI-compatible server.                               |
| `ollama` | `nomic-embed-text`         | none             | Talks to `http://127.0.0.1:11434`. Run `ollama pull nomic-embed-text` once.                                                    |
| `local`  | `bge-small-en-v1.5`        | none             | ONNX in-process via fastembed-rs. **Requires `--features local-embeddings` at compile time.** Downloads model on first run.    |

The cloud and Ollama paths share the same `OpenAiEmbeddings` HTTP
client behind the wire — they differ only in base URL and auth.

## Provider selection

```bash
# Use OpenAI (default if no flag and no YAML setting)
understandable embed

# Use Ollama
understandable embed --embed-provider ollama --embed-model bge-m3

# Use local ONNX (offline)
understandable embed --embed-provider local --embed-model bge-small
```

Resolution order, same as the rest of the binary:

1. CLI flag (`--embed-provider`).
2. `embeddings.provider` in `understandable.yaml`.
3. Default: `openai`.

## Re-running is cheap

Each row is keyed by a blake3 hash of the node's text (`name ::
summary :: tags`). When the hash matches the stored value, the row is
skipped — re-running on an unchanged graph costs zero API calls.

`analyze --incremental` automatically invalidates the embeddings of
changed/deleted nodes, so the next `embed` call refreshes only the
affected rows.

## Switching models — `--reset`

Each model has a fixed vector dimension. Switching to a model with a
different dimension is a hard reset; the previous vectors must be
discarded first:

```bash
understandable embed --reset --embed-model text-embedding-3-large
```

`--reset` drops every existing embedding for the named model before
the run. `--force` is similar but only re-embeds (it doesn't discard);
use it to refresh vectors when the underlying model has changed
behind a stable id.

:::tip
For multilingual content (Russian docs, Chinese comments, mixed-language
codebases), pick a multilingual model: `bge-m3` on Ollama or
`paraphrase-multilingual-mpnet-base-v2` on the local provider.
:::

## Concurrency

`embeddings.concurrency` (default 2) controls how many provider calls
run in parallel. Each task only does the I/O — the storage upserts
stay on the main task to avoid contending on the async-mutex-protected
state. Configure in YAML:

```yaml
embeddings:
  concurrency: 4
  batch_size: 32
```

`batch_size` is texts-per-call. The default of 32 is conservative;
OpenAI accepts up to 2048, Ollama tends to be slower per-batch.

## Examples

### First embed run

```bash
understandable embed
# embedded 3084/3084 node(s) into `text-embedding-3-small` (dim=1536)
```

### Switch from OpenAI to a multilingual Ollama model

```bash
ollama pull bge-m3
understandable embed --reset --embed-provider ollama --embed-model bge-m3
```

### Talk to an OpenAI-compatible server (e.g. vLLM, LiteLLM)

```bash
understandable embed --embed-provider openai \
  --embed-endpoint https://my-llm-gateway.example.com/v1
```

### Force a full refresh

```bash
understandable embed --force
```

## See also

- [`analyze`](./analyze) — build the graph first.
- [`init`](./init) — set provider, model, and concurrency in YAML.
- [Architecture](../architecture) — vector storage format.
