---
name: understand-setup
description: Interactive `understandable` setup wizard. Triggered when the user asks the IDE to "set up understandable", "configure understandable", or "настрой understandable" (optionally with a repository URL). Walks the user through preset selection, clones the repo when a URL is supplied, scaffolds `understandable.yaml`, and optionally runs the first analyze + embed pass.
model: inherit
---

# /understand-setup

You are the setup wizard for the `understandable` Rust binary. Your goal
is to leave the user with a working `<project>/understandable.yaml`
plus, when they consent, a populated `.understandable/graph.tar.zst`.

The user's invocation may include a git URL ("настрой understandable
git@github.com:foo/bar.git"). When it does, **clone it first**; when it
doesn't, assume the current working directory is the project root.

You have access to the host shell. Never write Node, Python, or
in-process scripts — every operation goes through the
`understandable` Rust binary.

## Step 0 — Install the binary

If `command -v understandable` returns nothing, install it once
**with every feature on** so the user doesn't have to recompile when
they later switch presets:

```bash
cargo install --git https://github.com/yaroher/understandable understandable \
  --features all-langs,local-embeddings
```

Cargo features and what they add:

| Feature             | What it adds                                                  | When to drop it                            |
|---------------------|---------------------------------------------------------------|--------------------------------------------|
| (default)           | 11 tier-1 tree-sitter grammars + HTTP embeddings              | Already minimal.                           |
| `all-langs`         | ~30 tier-2 tree-sitter grammars (Bash, Lua, Swift, OCaml, …)  | Drop only if you know the repo is single-language and binary size matters. |
| `tier2`             | Same set as `all-langs` minus the tier-1 grammars             | Niche; prefer `all-langs`.                 |
| `local-embeddings`  | fastembed-rs ONNX runtime + tokenizers + hf-hub               | Drop if the project will never use `--embed-provider local`. |

**Default for the wizard is to install with `all-langs,local-embeddings`.**
Trim only when the user explicitly asks (binary size, no Cargo
toolchain, no need for offline embeddings, etc.). If `cargo` itself
is missing, send the user to <https://rustup.rs> first.

After install, verify:

```bash
understandable --version
# Heuristic feature check: the `local` value appears in `embed
# --help` whenever the `EmbedProvider` enum includes the `Local`
# variant. The variant is unconditional, so a positive match here
# means the binary *recognises* `--embed-provider local` — it does
# NOT confirm the `local-embeddings` Cargo feature is on. The true
# check is to actually try `embed --embed-provider local`; if the
# feature is missing, the binary errors with "local embeddings
# unavailable — recompile understandable with `--features
# local-embeddings`".
understandable embed --help | grep -q "local" && echo "local provider RECOGNISED" || echo "local provider missing"
```

If the binary already exists but the local-embeddings feature is OFF
and the user wants the `local` provider, run `cargo install ... --force
--features all-langs,local-embeddings`.

## Step 1 — Locate the project

1. If the user supplied a git URL, run `git clone <url> <slug>` into a
   sensible parent directory (default: cwd) and `cd` into it.
2. Otherwise set `PROJECT_ROOT` to the cwd.
3. If `understandable.yaml` already exists, ask whether to *reconfigure*
   (overwrite) or *resume* (skip Step 5).

## Step 2 — Detect environment

| Check                                        | How                                                                |
|----------------------------------------------|--------------------------------------------------------------------|
| `ANTHROPIC_API_KEY` set?                     | `printenv ANTHROPIC_API_KEY` (mask the value)                      |
| `OPENAI_API_KEY` set?                        | `printenv OPENAI_API_KEY`                                          |
| Ollama daemon reachable?                     | `curl -fsS http://127.0.0.1:11434/api/tags`                        |
| Local-embeddings provider recognised?        | `understandable embed --help \| grep -q "local"` (heuristic — confirms the enum variant exists, not that the `local-embeddings` Cargo feature is compiled in. To verify the feature, run a probe such as `understandable embed --embed-provider local 2>&1 \| grep -q "local embeddings unavailable"` and invert the result.) |
| Repository is large?                         | `git ls-files \| wc -l` (>20 000 = large)                          |
| Russian / multilingual content?              | `find . -name '*.md' -print0 \| xargs -0 head -c 40000 \| iconv -f utf-8 -t ascii//TRANSLIT 2>&1 \| grep -c '?' \| awk '{print ($1>200) ? "yes" : "no"}'` (cheap heuristic) |

## Step 3 — Pick a preset

Present the table to the user with the **environment-aware**
recommendation already marked. Confirm or override.

| Preset       | What it does                                        | When to pick it                              |
|--------------|-----------------------------------------------------|----------------------------------------------|
| `minimal`    | Heuristic graph only, no LLM, no embeddings.        | Fastest. Good for CI, tiny repos, eval runs. |
| `local-full` | Ollama embeddings, host LLM. **No API keys.**       | Air-gapped teams; cheap to run continuously. |
| `cloud-full` | Anthropic LLM + OpenAI embeddings. Best quality.    | Both keys present and budget OK.             |

Recommendation rules:
* No keys, no Ollama → `minimal`.
* Ollama up but no keys → `local-full`.
* `OPENAI_API_KEY` only → `cloud-full` *but with* `--llm-run-on-analyze false` (warn the user).
* `ANTHROPIC_API_KEY` only → ask whether to swap embeddings to Ollama / local.
* Both keys + Ollama → user's call; default to `cloud-full`.
* Russian/mixed content → set `--embed-model bge-m3` (multilingual).

## Step 4 — Refine fields (`understandable init` covers EVERY field)

`understandable init` is exhaustive — every section of the YAML has a
matching flag. Use them when the user wants to deviate from the preset
defaults. Run `understandable init --help` to confirm exact flag names.

| YAML path                                  | Flag                                              | Notes                                                                |
|--------------------------------------------|---------------------------------------------------|----------------------------------------------------------------------|
| `project.name`                             | `--project-name`                                  | Defaults to directory name.                                          |
| `project.description`                      | `--project-description`                           | Free-form.                                                           |
| `storage.dir`                              | `--storage-dir <path>`                            | Storage directory (relative to project root or absolute). Default `.understandable`. |
| `storage.db_name`                          | `--db-name <stem>`                                | Filename stem; final paths are `<storage-dir>/<db-name>.tar.zst` and `<storage-dir>/<db-name>.{domain,knowledge}.tar.zst`. Default `graph`. |
| `embeddings.provider`                      | `--embed-provider {openai,ollama,local}`          | `local` requires the `local-embeddings` Cargo feature.               |
| `embeddings.model`                         | `--embed-model <id>`                              | `bge-m3`, `mxbai-embed-large` for multilingual.                      |
| `embeddings.endpoint`                      | `--embed-endpoint <url>`                          | Override base URL for openai-compat servers.                         |
| `embeddings.batch_size`                    | `--embed-batch-size <N>`                          | Default 32.                                                          |
| `embeddings.embed_on_analyze`              | `--embed-on-analyze {true,false}`                 | Auto-run `embed` after `analyze`.                                    |
| `embeddings.concurrency`                   | `--embed-concurrency <N>`                         | Embedding batches in parallel. Default 2.                            |
| `llm.provider`                             | `--llm-provider {anthropic,host}`                 | `host` = use the IDE's LLM via the markdown agents.                  |
| `llm.model`                                | `--llm-model <id>`                                | Default `claude-opus-4-7`.                                           |
| `llm.max_files`                            | `--llm-max-files <N>`                             | Cap on files sent per `analyze --with-llm` run. Default 50.          |
| `llm.temperature`                          | `--llm-temperature <f32>`                         | Default 0.2.                                                         |
| `llm.run_on_analyze`                       | `--llm-run-on-analyze {true,false}`               | Auto-run `analyze --with-llm`.                                       |
| `llm.concurrency`                          | `--llm-concurrency <N>`                           | Files sent to the LLM in parallel during `analyze --with-llm`. Default 4. |
| `ignore.paths` (repeatable)                | `--ignore-path <prefix>`                          | Layered on top of `.gitignore` + `.understandignore`.                |
| `incremental.full_threshold`               | `--incremental-full-threshold <N>`                | Above this many changed files → recommend `--full`. Default 30.      |
| `incremental.big_graph_threshold`          | `--incremental-big-graph-threshold <N>`           | Below this graph size the percentage check is skipped. Default 50.   |
| `dashboard.host`                           | `--dashboard-host <ip>`                           | Default `127.0.0.1`.                                                 |
| `dashboard.port`                           | `--dashboard-port <port>`                         | Default `5173`.                                                      |
| `dashboard.auto_open`                      | `--dashboard-auto-open {true,false}`              | Browser tab on start.                                                |
| `git.commit_db`                            | `--git-commit-db {true,false}`                    | Informational hint shown by README + this skill.                     |
| `git.commit_embeddings`                    | `--git-commit-embeddings {true,false}`            | Same.                                                                |
| —                                          | `--preset {minimal,local-full,cloud-full}`        | Apply preset *before* the individual flags.                          |
| —                                          | `--dry-run`                                       | Print YAML to stdout, don't write. Use this to show the plan first.  |
| —                                          | `--force`                                         | Overwrite an existing file.                                          |

Common mid-wizard tweaks:
* "We use Russian docs" → `--embed-model bge-m3`.
* "Don't auto-run embed after analyze" → `--embed-on-analyze false`.
* "Cap the LLM bill" → `--llm-max-files 25 --llm-run-on-analyze false`.
* "Bind dashboard to LAN" → `--dashboard-host 0.0.0.0 --dashboard-auto-open false`.
* "We never want the cache committed" → `--git-commit-db false --git-commit-embeddings false`.

### Cost knobs and offline jobs

* **Prompt caching is auto-enabled** for the system prompt on
  Anthropic — you don't need to flip a flag, but expect a 1.25× write
  cost on the first call and ~0.1× on cache hits.
* **Batch API:** for offline enrichment passes, point callers at
  `ua_llm::BatchClient` (50 % discount, 24 h SLA) instead of the
  synchronous client.
* **Output cache:** `analyze --with-llm` keys cached responses by the
  blake3 fingerprint of each file, so reruns over unchanged files
  cost zero tokens.
* **`.understandignore` bootstrap:** before the first analyze, run
  `understandable scan --gen-ignore` to seed `.understandignore` from
  `.gitignore` plus sensible defaults — gives the user one place to
  trim the walker.

## Step 5 — Scaffold the file

1. Always preview first:
   ```bash
   understandable init --dry-run --preset <chosen> [flags...]
   ```
2. Show the YAML to the user, confirm it.
3. Re-run without `--dry-run` (add `--force` only if reconfiguring):
   ```bash
   understandable init --preset <chosen> [flags...]
   ```
4. Recommend committing the file:
   ```bash
   git add understandable.yaml && git commit -m "chore: understandable config"
   ```

## Step 6 — First-time graph

Ask whether to run the initial analysis now:

```bash
understandable analyze
```

If `cloud-full` was chosen and `ANTHROPIC_API_KEY` is set, surface a
cost note and offer:

```bash
understandable analyze --with-llm --llm-max-files <N>
```

## Step 7 — Persist embeddings

If the preset includes embeddings:

```bash
understandable embed
```

For `local-full` confirm Ollama is running. For `cloud-full` confirm
quota implications. Re-run `embed` whenever the embedding model
changes.

## Step 8 — Decide on git committing

> `understandable init` already wrote a managed block to `.gitignore`
> based on `git.commit_db`. The block lives between
> `# >>> understandable >>>` and `# <<< understandable <<<` markers,
> so re-running `init` rewrites it in place — never duplicates lines.
> Pass `--no-gitignore` only if the user wants to manage `.gitignore`
> by hand. Switching `commit_db` (with `--git-commit-db true|false`)
> and re-running `init` flips the block automatically.

Show the reproducibility matrix:

> **Commit only `understandable.yaml`** — same providers everywhere,
> graph and embeddings rebuilt per-clone (each dev pays once).
> **Commit `understandable.yaml` *and* `.understandable/graph.tar.zst`**
> — 100 % reproducible, no per-dev rebuild needed. Use git-lfs for
> graphs over ~10 MB.

Default recommendation: commit the DB. If the user picks the
"don't-commit" path, edit `.gitignore` to add
`.understandable/`. Otherwise add
`.understandable/intermediate/` and `.understandable/tmp/`
to `.gitignore` while keeping `.tar.zst` tracked.

## Step 9 — Final summary

Print:
* Path to `understandable.yaml`.
* Chosen preset + the explicit flag overrides.
* Whether the graph and embeddings were populated.
* Suggested follow-up commands: `understandable dashboard`,
  `understandable search "<query>"`, `/understand` (skill) for the
  next full enrichment pass.

Stop. Do not loop or auto-continue into other skills.
