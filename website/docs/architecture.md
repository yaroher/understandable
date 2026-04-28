---
title: Architecture
sidebar_position: 99
---

# Architecture

`understandable` is an 8-crate Rust workspace plus a React dashboard,
held together by a single binary and a single archive format. This
page covers the moving parts and how they fit together.

## Workspace layout

```
crates/
  ua-core      — types, schema, validate
  ua-extract   — tree-sitter (tier1 + tier2) + line parsers
  ua-analyzer  — graph builder, layer detector, tour generator,
                 normalizer, domain extractor, wiki ingest
  ua-search    — nucleo fuzzy + chat-context builder
  ua-persist   — IndraDB + usearch + tar.zst + blake3 fingerprints
  ua-llm       — Anthropic + OpenAI-compat embeddings + Ollama clients
  ua-server    — axum server with the React bundle embedded via rust-embed
  ua-cli       — clap-based binary `understandable`
dashboard/     — React 19 + xyflow + zustand + tailwind v4
```

**`ua-core`** owns the canonical types (`KnowledgeGraph`, `GraphNode`,
`GraphEdge`, `ProjectSettings`) plus schema validation. Every other
crate consumes these types — the schema is the contract.

**`ua-extract`** runs tree-sitter and the line parsers. 11 tier-1
grammars (TypeScript/TSX, JavaScript, Python, Go, Rust, Java, Ruby,
PHP, C, C++, C#) are always on; ~30 tier-2 grammars (Bash, Lua, Zig,
Swift, OCaml, Elixir, …) sit behind `--features all-langs`. Custom
line parsers handle Dockerfile, Makefile, `.env`, INI.

**`ua-analyzer`** is the graph builder. It takes per-file metadata
from `ua-extract` plus optional LLM summaries and produces the typed
graph, layer assignments, and the heuristic tour.

**`ua-search`** wraps [`nucleo`][nucleo] for fuzzy matching against a
UTF-32 cache of vertex properties (`name_lower`, `summary_lower`,
`tags_text`). The chat-context builder turns a query into a graph
slice the LLM can answer over.

**`ua-persist`** is the storage layer: IndraDB MemoryDatastore +
usearch ANN index + tar.zst archive + blake3 fingerprints + the
`ignore` crate's walker.

**`ua-llm`** holds the Anthropic client, the OpenAI-compatible
embeddings client (used for both OpenAI and Ollama), and the
fastembed-rs ONNX bridge (gated on `--features local-embeddings`).
Prompt caching is auto-enabled for the Anthropic system prompt.

**`ua-server`** is an axum app. The React bundle is embedded with
[`rust-embed`][rust-embed] at compile time so the binary ships the
entire dashboard.

**`ua-cli`** is the clap-based entry point. Every subcommand
(`analyze`, `embed`, `dashboard`, `init`, …) lives in
`crates/ua-cli/src/commands/`.

## Storage format

Per-project state lives in `<project_root>/.understandable/`:

```
graph.tar.zst              — codebase graph (the default)
graph.domain.tar.zst       — optional, written by `understandable domain`
graph.knowledge.tar.zst    — optional, written by `understandable knowledge`
config.json                — {autoUpdate}
```

Each archive is **additive** — every artifact lives in one tarball,
so the graph ships as a single file rather than unbounded JSON
sprawl. Contents:

```text
meta.json            — schema version, project_root stamp, fingerprints,
                       layers, tour data, embedding meta (model → dim).
id_map.bincode       — business key → UUID v5 (deterministic).
graph.msgpack        — IndraDB MemoryDatastore msgpack snapshot.
embeddings.bin       — raw f32 vectors with a small bincode header.
```

Vertex IDs derive from `Uuid::new_v5(<fixed namespace>, <business
key>)` so they're stable across rebuilds — re-running `analyze` on
the same input produces the same UUIDs.

The raw f32 vectors in `embeddings.bin` are the source of truth; the
[`usearch`][usearch] HNSW index sits on top, rebuilt lazily after
mutations. The index is throwaway — a future `usearch` major bump
won't strand the data.

## Tree-sitter coverage

- **Tier 1** (default, full extraction + call graph): TypeScript /
  TSX, JavaScript, Python, Go, Rust, Java, Ruby, PHP, C, C++, C#.
- **Tier 2** (`--features all-langs`): Bash, Lua, Zig, Dart, Swift,
  Scala, Haskell, OCaml, Elixir, Erlang, Elm, Julia, Scheme, Solidity,
  Perl, Fortran, D, F#, Groovy, Objective-C, CUDA, GLSL, HLSL,
  Verilog, VHDL, CMake, Make, Nix, Vim script, Fish, jq, HCL.
- **Tier 3** (metadata only): HTML, CSS, JSON, YAML, TOML, XML,
  Markdown, regex.

## LLM and embeddings

| Layer       | Providers                                         |
|-------------|---------------------------------------------------|
| LLM         | Anthropic (default), `host` (delegate to IDE)     |
| Embeddings  | OpenAI (default), Ollama, local fastembed-rs ONNX |

OpenAI and Ollama share the same `OpenAiEmbeddings` HTTP client —
they differ only in base URL and auth.

### Cost knobs

| Knob               | What it does                                                                                  |
|--------------------|-----------------------------------------------------------------------------------------------|
| Prompt caching     | Auto-on for the Anthropic system prompt. First call writes (1.25×), reads ~0.1×.              |
| Batch API          | `ua_llm::BatchClient` — 50 % discount, 24 h SLA. Use for offline enrichment.                  |
| Output cache       | `analyze --with-llm` keys responses by per-file blake3. Re-runs over unchanged files = free.  |
| Concurrency caps   | `llm.concurrency` (default 4), `embeddings.concurrency` (default 2). Bounded by `Semaphore`.  |

## Search

Two paths over the same graph:

- **`search "<query>"`** — token-AND substring scan against
  `name_lower` / `summary_lower` / `tags_text` columns, re-ranked by
  [`nucleo`][nucleo] over a UTF-32 cache.
- **`search --semantic "<query>"`** — cosine similarity over the
  usearch HNSW index. Requires a populated `embeddings.bin`.

Both are in-memory and complete in milliseconds on a 10k-node graph.

## Dashboard

`ua-server` boots an axum app on `127.0.0.1:5173` (configurable). The
React 19 + xyflow + zustand + tailwind v4 frontend lives in
`dashboard/`; `pnpm build` produces a static bundle that
`rust-embed` slurps into the binary at compile time. There's no
separate frontend deploy.

## Plug-in surface

The binary is the substrate. The IDE-side experience is
markdown-only:

- **9 IDE manifests** — `.claude-plugin/`, `.cursor-plugin/`,
  `.copilot-plugin/`, `.codex/`, `.opencode/`, `.openclaw/`,
  `.gemini/`, `.pi/`, `.antigravity/`, `.vscode/`. Each adapts the
  same skill/agent/hook bundle to a different IDE's manifest schema.
- **`agents/`** — 9 markdown agents (file-analyzer,
  architecture-analyzer, …) the IDE invokes when the user asks a
  question.
- **`skills/`** — 8 markdown slash-commands (`/understand`,
  `/understand-setup`, `/install-understandable`, …).
- **`hooks/`** — post-commit hook + prompt that runs `analyze
  --incremental --plan-only` and decides whether to invoke the LLM.

## Data flow

```
                      ┌──────────────────────────┐
                      │  source repo (cwd)       │
                      └────────────┬─────────────┘
                                   │
                                   ▼
            ┌──────────────────────────────────────────┐
            │  ua-extract  (tree-sitter, line parsers) │
            └────────────────────┬─────────────────────┘
                                 │ FileMeta
                                 ▼
            ┌──────────────────────────────────────────┐
            │  ua-analyzer  (GraphBuilder, layers,     │
            │                tour, optional LLM enrich)│
            └────────────────────┬─────────────────────┘
                                 │ KnowledgeGraph
                                 ▼
            ┌──────────────────────────────────────────┐
            │  ua-persist  (IndraDB + usearch HNSW)    │
            └────────────────────┬─────────────────────┘
                                 │
                                 ▼
                    ┌────────────────────────┐
                    │ .understandable/       │
                    │   graph.tar.zst        │
                    │   graph.domain.tar.zst │
                    │   graph.knowledge…     │
                    └──────┬──────────┬──────┘
                           │          │
                ┌──────────▼──┐    ┌──▼─────────────────────┐
                │ ua-server   │    │ IDE agents / skills    │
                │ (dashboard) │    │ (file-analyzer, …)     │
                └─────────────┘    └────────────────────────┘
```

The archive is the contract. The dashboard reads it; the IDE-side
agents read it. Anyone can write a new consumer that opens
`graph.tar.zst` and walks the graph — it's just msgpack + bincode +
raw f32.

[nucleo]: https://crates.io/crates/nucleo-matcher
[usearch]: https://crates.io/crates/usearch
[rust-embed]: https://crates.io/crates/rust-embed
