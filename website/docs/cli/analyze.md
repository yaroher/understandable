---
title: analyze
sidebar_position: 1
---

# `understandable analyze`

The core build step. Walks the project, runs tree-sitter, assembles
the typed graph, and persists it to `.understandable/graph.tar.zst`.

## Synopsis

```bash
understandable analyze [--full | --incremental] [--with-llm] [--review] \
                       [--auto-update | --no-auto-update] \
                       [--plan-only] [--scan-only]
```

## Flags

### Pipeline mode

| Flag              | Effect                                                                             |
|-------------------|------------------------------------------------------------------------------------|
| `--full`          | Force a full rebuild even when a cached graph exists.                              |
| `--incremental`   | Skip files whose blake3 hash matches stored fingerprints; rebuild only the slice. |
| `--plan-only`     | Dry-run: print a JSON change plan to stdout, do not persist. Requires `--incremental`. |
| `--scan-only`     | Emit a project-scanner JSON document and exit. Bypasses the analyzer entirely.    |

### LLM enrichment

| Flag                   | Effect                                                                                 |
|------------------------|----------------------------------------------------------------------------------------|
| `--with-llm`           | Per-file summary/tags/complexity from the configured LLM provider.                     |
| `--llm-model <id>`     | Override `llm.model`. Default: `claude-opus-4-7`.                                      |
| `--llm-max-files <N>`  | Cap files sent to the LLM. `0` = use `llm.max_files` (default 50).                     |
| `--llm-concurrency <N>`| Files sent in parallel. Set in YAML; default 4.                                        |
| `--llm-temperature <f>`| Sampling temperature. Set in YAML; default 0.2.                                        |

When `llm.provider == "host"`, `--with-llm` is delegated to the
IDE-side markdown agents and the binary skips its own LLM loop.

### Hooks and feedback

| Flag                | Effect                                                                  |
|---------------------|-------------------------------------------------------------------------|
| `--review`          | Print a richer one-line summary after analyze (file counts, deletions). |
| `--auto-update`     | Write `.understandable/auto-update.signal` to nudge IDE-side agents.    |
| `--no-auto-update`  | Suppress the nudge file even if a project default would write it.       |
| `--name <project>`  | Override the project name (CLI > settings > directory basename).        |

## Concurrency model

`analyze --with-llm` uses a `tokio::JoinSet` bounded by a
`tokio::sync::Semaphore` whose capacity is `llm.concurrency` (default
4). Each file becomes one task; the semaphore caps simultaneous
in-flight LLM calls so a 10k-file repo can't blow up your token
budget or the provider's rate limit.

Embeddings (when `embed_on_analyze: true`) follow the same pattern
with `embeddings.concurrency` (default 2).

## Output cache

`analyze --with-llm` keys cached responses by:

- `file_hash` — blake3 of the file contents.
- `prompt_hash` — blake3 of the system prompt + model id.

A re-run on unchanged files costs **zero tokens**. The cache lives
inside `graph.tar.zst` next to the graph itself.

:::tip
Combine the output cache with prompt caching (auto-on for the
Anthropic system prompt) and the Batch API (50 % discount, 24 h SLA)
— see the cost knobs section in
[architecture](../architecture).
:::

## Settings cascade

Every flag has a YAML equivalent. Resolution order:

1. **CLI flag** (highest priority).
2. **`understandable.yaml`** in the project root.
3. **Built-in defaults** (lowest priority).

So `--llm-max-files 25` always wins over `llm.max_files: 50` in YAML
which always wins over the built-in default of 50.

## Examples

### First analyze, no LLM

```bash
understandable analyze
```

Heuristic-only graph. Fastest. Re-run after every meaningful change.

### Incremental rebuild after a commit

```bash
understandable analyze --incremental --review
```

The post-commit hook uses this combination — only re-extracts files
whose blake3 changed and prints a one-line diff summary.

### LLM enrichment with budget cap

```bash
export ANTHROPIC_API_KEY=sk-ant-...
understandable analyze --with-llm --llm-max-files 25 --llm-concurrency 2
```

Caps the run at 25 files and 2 simultaneous calls. Re-running on
unchanged files reuses the output cache.

### Plan-only preview (dry-run)

```bash
understandable analyze --incremental --plan-only | jq .
```

Outputs a JSON change plan with `files_to_reanalyze`,
`rerun_architecture`, `rerun_tour`. Used by the post-commit hook to
decide whether to invoke the LLM at all.

## See also

- [`init`](./init) — every YAML field has a flag.
- [`embed`](./embed) — semantic search index.
- [Architecture](../architecture) — storage layout and data flow.
