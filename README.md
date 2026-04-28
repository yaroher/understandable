# understandable

Codebase comprehension as a single Rust binary. Turns any source repo
into an interactive knowledge graph plus a guided tour, served from a
local web dashboard.

* **Native tree-sitter parsing** — 11 tier-1 languages with full structural
  + call-graph extraction, ~30 tier-2 languages behind feature flags.
* **In-memory IndraDB graph + hnsw_rs ANN** packed into a single
  `graph.tar.zst` file — graphs ship as one archive instead of
  unbounded JSON, and vector search uses HNSW instead of a linear scan.
* **Embedded React dashboard** — same dark luxury UI as the original,
  served by axum and bundled into the binary at compile time.
* **Cross-IDE plugin** — markdown skills + agents work in Claude Code,
  Cursor, Copilot, Codex, OpenCode, OpenClaw, Gemini, Pi, Antigravity,
  and VS Code via per-IDE manifest dirs.

## Install

The recommended default brings every Cargo feature in:

```bash
cargo install --git https://github.com/yaroher/understandable understandable \
  --features all-langs,local-embeddings
```

That gives you all 40+ tree-sitter grammars and offline ONNX embeddings
in one binary (~80 MB). Trim down only when you know you don't need a
slice:

```bash
# Slim builds (only when you know you won't need the dropped slice):
cargo install --git https://github.com/yaroher/understandable understandable                   # tier-1 langs only, OpenAI/Ollama embeddings only
cargo install --git https://github.com/yaroher/understandable understandable --features all-langs        # add tier-2 grammars, no local embeddings
cargo install --git https://github.com/yaroher/understandable understandable --features local-embeddings # add ONNX embeddings, only tier-1 grammars
```

| Feature             | What it adds                                                        | Binary cost          |
|---------------------|---------------------------------------------------------------------|----------------------|
| (default)           | 11 tier-1 tree-sitter grammars, OpenAI/Ollama embeddings via HTTP   | ~40 MB               |
| `all-langs`         | + ~30 tier-2 tree-sitter grammars                                   | ~+25 MB              |
| `tier2`             | same as `all-langs` minus tier-1 (rarely used in isolation)         | depends on subset    |
| `local-embeddings`  | + fastembed-rs ONNX runtime + tokenizers + hf-hub                   | ~+30 MB on disk; downloads model on first run |

The binary is named `understandable`. The markdown skills/agents shell
out to it — the same binary serves both the CLI and the embedded web
dashboard.

## Project settings (`understandable.yaml`)

Every subcommand reads `<project_root>/understandable.yaml` (or `.yml`)
before applying built-in defaults so the team can pin embedding models,
dashboard ports, and incremental thresholds in git. CLI flags override
the file; the file overrides the built-in defaults.

```bash
understandable init                                                  # scaffold defaults
understandable init --preset minimal                                 # heuristic only, no LLM, no embeddings
understandable init --preset local-full                              # Ollama embeddings, host LLM (no API keys)
understandable init --preset cloud-full                              # Anthropic + OpenAI (best quality)
understandable init --dry-run --preset cloud-full                    # print resulting YAML, don't write
```

For an LLM-driven setup, ask your IDE assistant to "set up
understandable" or "настрой understandable [git URL]" — the
[`understand-setup` skill](skills/understand-setup/SKILL.md) walks
the user through environment detection, preset selection, and the
first analyze+embed pass.

Generated file:

```yaml
version: 1
project:
  name: null
  description: null
storage:
  dir: .understandable        # project-relative or absolute
  db_name: graph              # → <dir>/graph.tar.zst, <dir>/graph.domain.tar.zst, …
embeddings:
  provider: openai            # openai | ollama | local
  model: null                 # null → provider default
  endpoint: null              # override base URL for openai-compat servers
  batch_size: 32
  embed_on_analyze: false
  concurrency: 2              # embedding batches in parallel
llm:
  provider: anthropic
  model: null                 # null → claude-opus-4-7
  max_files: 50
  temperature: 0.2
  run_on_analyze: false
  concurrency: 4              # files sent to the LLM in parallel
ignore:
  paths: []
incremental:
  full_threshold: 30
  big_graph_threshold: 50
dashboard:
  host: 127.0.0.1
  port: 5173
  auto_open: true
git:
  commit_db: true
  commit_embeddings: true
```

### Reproducibility

| What you commit                                | Reproducibility |
|------------------------------------------------|-----------------|
| `understandable.yaml` only                     | Same providers, thresholds, dashboard port. Graph + embeddings depend on each dev's LLM responses. |
| `understandable.yaml` + `.understandable/graph.tar.zst` | **100 %** — every dev opens the same graph + embeddings. Recommended. |

The `.tar.zst` carries the IndraDB MessagePack snapshot, the raw f32
embedding vectors, and a side-car `meta.json` with fingerprints,
layers, and tour data — all in one atomic archive. Vertex IDs derive
from `Uuid::new_v5(<fixed namespace>, <business key>)` so they're
stable across rebuilds.

`understandable init` writes (or rewrites) a managed block in
`<project>/.gitignore` based on `git.commit_db`. The block sits
between `# >>> understandable >>>` and `# <<< understandable <<<`
markers — re-running `init` updates it in place without disturbing
your hand-written entries. Pass `--no-gitignore` to opt out.

`.gitignore` block in *commit-DB* mode (recommended, default):

```gitignore
# >>> understandable >>>
# managed by `understandable init` — leave the DB tracked.
.understandable/intermediate/
.understandable/tmp/
# <<< understandable <<<
```

For repos where the graph crosses ~10 MB, use git-lfs:

```bash
git lfs install
git lfs track ".understandable/*.tar.zst"
git add .gitattributes
```

In *don't-commit-DB* mode (`--git-commit-db false`) the block becomes:

```gitignore
# >>> understandable >>>
# managed by `understandable init` — DB stays out of git.
.understandable/
# <<< understandable <<<
```

Every dev then runs `understandable analyze && understandable embed`
once per checkout.

### Project-wide ignore-respect

Every file walker in the binary (`analyze`, `fingerprint`, the
incremental change-detector, the wiki ingest behind `knowledge`)
goes through the [`ignore`](https://docs.rs/ignore) crate with
`.gitignore`, `.git/info/exclude`, the global git ignore, and a
project-local `.understandignore` all stacked. Common `.idea/`,
`node_modules/`, `target/`, `dist/`, `.venv/` will be skipped
automatically as long as they're listed in any of those files.

## Quick start

```bash
cd /path/to/your/repo

# 0. (optional) seed `.understandignore` from `.gitignore` + sensible defaults
understandable scan --gen-ignore

# 1. build the graph (deterministic substrate; LLM enrichment lives in
#    the IDE-side agents at agents/file-analyzer.md and friends)
understandable analyze

# 2. open the dashboard (`--kind {codebase,domain,knowledge}` selects
#    which graph to serve; default is `codebase`)
understandable dashboard --port 5173
understandable dashboard --kind domain --port 5174     # serve the domain graph alongside

# 3. inspect from the terminal
understandable search "auth"
understandable explain src/auth.ts:login
understandable diff
understandable onboard
```

## Cost knobs

| Knob                              | What it does                                                                       |
|-----------------------------------|------------------------------------------------------------------------------------|
| `--llm-concurrency <N>`           | Files sent to the LLM in parallel during `analyze --with-llm`. Default 4.          |
| `--embed-concurrency <N>`         | Embedding batches in parallel. Default 2.                                          |
| Prompt caching                    | Auto-on for the system prompt on Anthropic — first call writes (1.25×), reads ~0.1×. |
| Batch API (`ua_llm::BatchClient`) | 50 % discount, 24 h SLA. Use for offline enrichment passes.                        |
| Output cache                      | `analyze --with-llm` keys responses by per-file blake3 fingerprint; reruns over unchanged files cost zero tokens. |

## Subcommands

| Command                                    | What it does                                          |
|--------------------------------------------|-------------------------------------------------------|
| `analyze [--full] [--incremental] [--plan-only]` | scan → extract → graph → persist                      |
| `scan [--gen-ignore]`                      | bootstrap `.understandignore` from `.gitignore` defaults |
| `dashboard [--kind {codebase,domain,knowledge}] [--port] [--host] [--no-open]` | embedded React UI + JSON API on localhost             |
| `chat <query>`                             | search graph, expand 1 hop, render LLM-ready prompt   |
| `diff [--files]`                           | map current git changes to nodes + risk assessment    |
| `explain <path | path:symbol>`            | deep-dive prompt for a file or function               |
| `onboard [--out file.md]`                  | markdown onboarding guide derived from the graph      |
| `domain [--full]`                          | derive domain/flow/step substrate (saves separately)  |
| `knowledge <wiki>`                         | Karpathy-wiki ingest → article/topic graph            |
| `extract --batch in.json --out out.json`   | tree-sitter structural extraction (used by agents)    |
| `merge --kind {file,subdomain,knowledge}`  | merge intermediate JSON outputs (used by agents)      |
| `validate`                                 | ID/edge/weight consistency over the persisted graph   |
| `fingerprint`                              | recompute file hashes; powers `analyze --incremental` |
| `export [--kind] [--pretty] [--out]`       | dump persisted graph as JSON                          |
| `import [--kind] [--in]`                   | replace persisted graph from JSON                     |
| `search <query> [--limit]`                 | substring/LIKE prefilter against the persisted graph  |
| `embed [--embed-provider] [--reset]`       | bulk-embed graph nodes; vectors persisted in `embeddings.bin` so `search --semantic` doesn't re-embed |

`--path <PROJECT_ROOT>` is global and defaults to the current directory.
`-v` / `-vv` / `-vvv` raise log verbosity.

## Embedding pipeline

Vectors live in the same `graph.tar.zst` as the graph. The header inside
`embeddings.bin` records `(node_id, model, dim, text_hash, updated_at)`
per row; the raw f32 payload is the source of truth. An hnsw_rs HNSW
index sits on top, rebuilt lazily after mutations. The dimension is
fixed per model — switching to a model with a different vector size
requires `understandable embed --reset --embed-model <new>`.

```bash
understandable analyze                                     # build the graph
understandable embed                                       # populate embeddings (OpenAI by default)
understandable embed --embed-provider ollama --embed-model bge-m3
understandable embed --embed-provider local --embed-model bge-small  # offline ONNX

understandable search --semantic "user authentication flow"
```

`embed` skips nodes whose `text_hash` matches the previous run, so
re-running is cheap. `analyze --incremental` automatically forgets the
embeddings of changed/deleted nodes so the next `embed` call refreshes
only the affected rows.

Cosine similarity uses an HNSW index from
[`hnsw_rs`](https://crates.io/crates/hnsw_rs) (`DistCosine`,
`simdeez_f` feature). The index is rebuilt lazily after mutations:
`upsert_node_embedding` flips a dirty flag and the next `vector_top_k`
calls `Hnsw::parallel_insert` over every row for that model. The raw
f32 vectors in `embeddings.bin` are the source of truth, so the index
is throwaway — a future `hnsw_rs` major bump won't strand the data.

## Embedding providers

`understandable search --semantic` ranks candidates by cosine
similarity over an embedding space. Pick a provider via
`--embed-provider`:

| Provider | Default model              | Auth                | Notes                                                    |
|----------|----------------------------|---------------------|----------------------------------------------------------|
| `openai` | `text-embedding-3-small`   | `OPENAI_API_KEY`    | Default. Hits `api.openai.com`. Pass `--embed-endpoint` to talk to any OpenAI-compatible server. |
| `ollama` | `nomic-embed-text`         | none                | Talks to `http://127.0.0.1:11434` out of the box. `ollama pull nomic-embed-text` first. |
| `local`  | `bge-small-en-v1.5`        | none                | ONNX in-process via fastembed-rs. **Requires `--features local-embeddings` at compile time.** Downloads model on first run (~120 MB cached under `~/.cache/fastembed`). |

Local-embeddings adds ~30-100 MB of native deps (`ort`, `tokenizers`,
`hf-hub`) so the feature is opt-in. Cloud and Ollama paths share the
same `OpenAiEmbeddings` client behind the wire.

## Storage layout

Per project, in `<project_root>/.understandable/`:

```
graph.tar.zst              # IndraDB msgpack + embeddings + meta — codebase graph
graph.domain.tar.zst       # optional, written by `understandable domain`
graph.knowledge.tar.zst    # optional, written by `understandable knowledge`
config.json                # {autoUpdate}
```

Each archive contains:

```text
meta.json            # schema version, project_root stamp, fingerprints,
                     # layers, tour, embedding meta (model → dim).
id_map.bincode       # business key → UUID v5 (deterministic).
graph.msgpack        # IndraDB MemoryDatastore msgpack snapshot.
embeddings.bin       # raw f32 vectors with a small bincode header.
```

Search is in-memory: a token-AND scan over IndraDB vertex properties
(`name_lower` / `summary_lower` / `tags_text`) re-ranked by
[`nucleo`](https://crates.io/crates/nucleo-matcher) inside
[`ua-search`](crates/ua-search).

## Workspace layout

```
crates/
  ua-core      — types, schema, validate
  ua-extract   — tree-sitter (tier1 + tier2) + line parsers (Dockerfile,
                 Makefile, .env, .ini) + language + framework registries
  ua-analyzer  — graph builder, layer detector, tour generator,
                 normalizer, domain extractor, wiki ingest
  ua-search    — nucleo fuzzy + chat-context builder
  ua-persist   — IndraDB + hnsw_rs + tar.zst + blake3 fingerprints + ignore + staleness
  ua-llm       — placeholder for upcoming Anthropic / OpenAI clients
  ua-server    — axum server with the React bundle embedded via rust-embed
  ua-cli       — clap-based binary `understandable`
dashboard/     — React 19 + xyflow + zustand + tailwind v4
skills/        — 8 markdown slash-commands
agents/        — 9 markdown agents (file-analyzer, architecture-analyzer, ...)
hooks/         — auto-update post-commit hook + prompt
.claude-plugin/, .cursor-plugin/, .copilot-plugin/, .codex/, .opencode/,
.openclaw/, .gemini/, .pi/, .antigravity/, .vscode/   — per-IDE manifests
```

## Language coverage

* **Tier 1 (default, full extraction + call graph):** TypeScript / TSX,
  JavaScript, Python, Go, Rust, Java, Ruby, PHP, C, C++, C#.
* **Tier 2 (`--features tier2`):** Bash, Lua, Zig, Dart, Swift, Scala,
  Haskell, OCaml, Elixir, Erlang, Elm, Julia, Scheme, Solidity, Perl,
  Fortran, D, F#, Groovy, Objective-C, CUDA, GLSL, HLSL, Verilog, VHDL,
  CMake, Make, Nix, Vim script, Fish, jq, HCL.
* **Custom line parsers:** Dockerfile (multi-stage, EXPOSE, RUN/CMD),
  Makefile (`:=`/`?=`/`+=`/`=` + targets), `.env`, INI/`.cfg`.
* **Tier 3 (metadata only, no AST):** HTML, CSS, JSON, YAML, TOML, XML,
  Markdown, regex.

`all-langs` umbrella feature pulls every available grammar — expect
2–5 minutes of compile time the first run.

## Development

```bash
cargo test --workspace                  # ua-core, ua-extract, ua-analyzer,
                                        # ua-search, ua-persist
cargo test -p ua-extract --features all-langs --no-default-features
cd dashboard && pnpm install && pnpm build   # rebuild the embedded UI
```

When releasing, bump the version in **four files** (kept in sync):

1. `Cargo.toml` (workspace)
2. `.claude-plugin/plugin.json`
3. `.cursor-plugin/plugin.json`
4. `.copilot-plugin/plugin.json`

`.claude-plugin/marketplace.json` has no `version` field — leave it.

## License

MIT. Original architecture by [Lum1104](https://github.com/Lum1104).
Rust port by [@yaroher](https://github.com/yaroher).

[orig]: https://github.com/Lum1104/understandable
