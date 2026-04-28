---
title: Your First Graph
sidebar_position: 2
---

# Your First Graph

A concrete end-to-end walkthrough: scaffold a config, build the
knowledge graph, embed it, and open the dashboard. Takes a couple of
minutes on a small project.

Prerequisite: the `understandable` binary on your `PATH`. See
[Install](./install) if you haven't installed it yet.

## 1. Pick a real project

```bash
cd /path/to/your/repo
```

Any source repo works. The walker honours `.gitignore`, so vendored
dependencies, build artifacts, and `node_modules/` are skipped
automatically.

## 2. Scaffold the config — `understandable init`

`init` writes `understandable.yaml` in the project root and updates
`.gitignore` with a managed block. Pick a preset based on what you
have available:

```bash
# Heuristic only — no LLM, no embeddings. Fastest. CI-friendly.
understandable init --preset minimal

# Ollama embeddings + host LLM (no API keys, runs entirely local).
understandable init --preset local-full

# Anthropic LLM + OpenAI embeddings (best quality; needs both keys).
understandable init --preset cloud-full
```

Preview before writing with `--dry-run`:

```bash
understandable init --dry-run --preset cloud-full
```

See [`init`](../cli/init) for every flag (every YAML field has one).

:::tip
Inside an IDE that loads the `understand-setup` skill, just say
**"set up understandable"** / **"настрой understandable"** and the
wizard handles environment detection, preset selection, and the
first analyze + embed pass for you.
:::

## 3. Bootstrap ignore rules — `scan --gen-ignore`

```bash
understandable scan --gen-ignore
```

This seeds `.understandignore` from your `.gitignore` plus a list of
sensible defaults (`target/`, `.venv/`, `dist/`, `.idea/`, …). You'll
edit this file later to trim noisy directories.

## 4. Build the graph — `understandable analyze`

```bash
understandable analyze
```

Output:

```
analysis complete: 412 files → 3,084 nodes, 5,221 edges, 7 layers, 12 tour steps
```

What happened:
1. The walker discovered every source file, honouring `.gitignore` +
   `.understandignore` + `ignore.paths` from `understandable.yaml`.
2. Tree-sitter extracted symbols, calls, imports, and structural
   metadata.
3. `GraphBuilder` assembled the typed graph.
4. The layer detector and heuristic tour generator ran.
5. The whole thing was packed into `.understandable/graph.tar.zst`.

## 5. (Optional) LLM enrichment — `analyze --with-llm`

```bash
export ANTHROPIC_API_KEY=sk-ant-...
understandable analyze --with-llm --llm-max-files 50
```

Each file gets a one-shot summary + tags + complexity rating. Output
is cached by per-file blake3 fingerprint, so re-running on unchanged
files is free. See [`analyze`](../cli/analyze) for cost knobs.

## 6. Build the semantic search index — `understandable embed`

```bash
understandable embed
```

Embeds every node's `name :: summary :: tags` text and stores the
vectors inside the same `graph.tar.zst`. Re-running is cheap — rows
whose text hash hasn't changed are skipped.

## 7. Open the dashboard — `understandable dashboard`

```bash
understandable dashboard
```

Opens `http://127.0.0.1:5173` in your browser. You get:

- **Graph view** — pan, zoom, click any node to see its neighbours.
- **Search box** — substring + fuzzy match by default; use the
  semantic toggle to rank by cosine similarity.
- **Layer panel** — the auto-detected architectural layers.
- **Tour mode** — step through the heuristic tour like a guided
  introduction.

## 8. Try the search

In the dashboard, search for **"auth"** or anything you know exists
in your codebase. Click a node. The neighbour panel lists every
node connected by an edge — call sites, imports, references.

From the terminal:

```bash
understandable search "auth"
understandable search --semantic "user authentication flow"
understandable explain src/auth.ts:login
```

## Next steps

- [`init`](../cli/init) — every config knob.
- [`analyze`](../cli/analyze) — flags, cost knobs, incremental mode.
- [`embed`](../cli/embed) — provider selection, model swaps.
- [`dashboard`](../cli/dashboard) — multi-graph view, LAN exposure.
- [Architecture](../architecture) — what's actually inside the
  `tar.zst`.
