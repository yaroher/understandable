---
slug: /
title: What is understandable?
sidebar_position: 1
---

# What is understandable?

**Codebase comprehension as a single Rust binary.** Point it at any
source repository and you get back a navigable knowledge graph, a
guided tour, semantic search, and a local web dashboard — all served
from one ~40 MB executable.

## Why it exists

`understandable` is a Rust port of the original TypeScript
[`Understand-Anything`][orig] project. The TS prototype proved the
idea worked; the Rust rewrite makes it cheap to run.

What the port buys you:

- **Single binary, zero runtime deps.** No `node_modules`, no Python
  venv, no language server. The dashboard's React UI is
  embedded into the binary at compile time via `rust-embed`.
- **Native tree-sitter.** Parsers run in-process — no IPC, no JSON
  shuttling between processes. Tier-1 grammars cover 11 languages out
  of the box; ~30 more sit behind `--features all-langs`.
- **Performance.** A 10k-file repo analyzes in seconds. Embedding,
  search, and graph traversal all stay in-process against an in-memory
  IndraDB store backed by a `tar.zst` archive.
- **Deterministic substrate.** Vertex IDs derive from `Uuid::new_v5`
  over stable business keys. Re-running `analyze` on the same input
  produces the same graph.

## What you get

After one `understandable analyze` you have:

- **A typed knowledge graph** of files, symbols, modules, layers, and
  their dependencies.
- **A semantic search index** (when you also run `understandable
  embed`) — ranks nodes by cosine similarity over OpenAI, Ollama, or
  fully-offline ONNX embeddings.
- **A guided tour** — a heuristic ordering of nodes that walks a
  newcomer through the project's structure.
- **An interactive dashboard** at `http://127.0.0.1:5173` with a node
  graph, neighbour explorer, and search box.
- **LLM-aware enrichment** — optional per-file summaries, tags, and
  complexity ratings via Anthropic, OpenAI-compat servers, or your
  IDE's host LLM (Claude Code, Cursor, Copilot, …).

## Quick start

```bash
cargo install --git https://github.com/yaroher/understandable understandable \
  --features all-langs,local-embeddings

cd /path/to/your/repo
understandable init --preset cloud-full
understandable analyze
understandable dashboard
```

See [Install](./getting-started/install) for the full set of install
paths and [Your First Graph](./getting-started/first-graph) for the
guided walkthrough.

## Next steps

- [Install](./getting-started/install) — three install paths plus
  per-platform notes.
- [Your First Graph](./getting-started/first-graph) — concrete
  end-to-end walkthrough.
- [Architecture](./architecture) — workspace layout, storage format,
  data flow diagram.
- CLI reference: [`init`](./cli/init), [`analyze`](./cli/analyze),
  [`embed`](./cli/embed), [`dashboard`](./cli/dashboard).

[orig]: https://github.com/Lum1104/understandable
