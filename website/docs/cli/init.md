---
title: init
sidebar_position: 4
---

# `understandable init`

Scaffolds (or updates) `understandable.yaml` in the project root.
Every field of the config is reachable via a flag, so the
`understand-setup` LLM-led wizard can build any combination
deterministically.

## Synopsis

```bash
understandable init [--preset {minimal,local-full,cloud-full}] \
                    [--dry-run] [--force] [--no-merge] [--no-backup] \
                    [--no-gitignore] \
                    [--<any-field>=<value>...]
```

## Presets

Three opinionated bundles cover the common deployment shapes:

| Preset       | Embeddings              | LLM                | When to use                                       |
|--------------|-------------------------|--------------------|---------------------------------------------------|
| `minimal`    | none                    | none               | CI, eval runs, tiny repos. Heuristic only.        |
| `local-full` | Ollama (`nomic-embed-text`) | Host LLM (no API key) | Air-gapped teams; cheap to run continuously. |
| `cloud-full` | OpenAI (`text-embedding-3-small`) | Anthropic (`claude-opus-4-7`) | Best quality. Needs both API keys.        |

Apply order: **`recommended() → preset → individual flags`**, so any
explicit flag wins over the preset.

```bash
understandable init --preset cloud-full
understandable init --preset local-full --embed-model bge-m3   # multilingual override
```

## Every YAML field is a flag

Every section of `ProjectSettings` has a matching flag. Mapping:

| YAML path                          | Flag                                   |
|------------------------------------|----------------------------------------|
| `project.name`                     | `--project-name <s>`                   |
| `project.description`              | `--project-description <s>`            |
| `storage.dir`                      | `--storage-dir <path>`                 |
| `storage.db_name`                  | `--db-name <stem>`                     |
| `embeddings.provider`              | `--embed-provider {openai,ollama,local}` |
| `embeddings.model`                 | `--embed-model <id>`                   |
| `embeddings.endpoint`              | `--embed-endpoint <url>`               |
| `embeddings.batch_size`            | `--embed-batch-size <N>`               |
| `embeddings.embed_on_analyze`      | `--embed-on-analyze {true,false}`      |
| `embeddings.concurrency`           | `--embed-concurrency <N>`              |
| `llm.provider`                     | `--llm-provider {anthropic,host}`      |
| `llm.model`                        | `--llm-model <id>`                     |
| `llm.max_files`                    | `--llm-max-files <N>`                  |
| `llm.temperature`                  | `--llm-temperature <f32>`              |
| `llm.run_on_analyze`               | `--llm-run-on-analyze {true,false}`    |
| `llm.concurrency`                  | `--llm-concurrency <N>`                |
| `ignore.paths` (repeatable)        | `--ignore-path <prefix>`               |
| `incremental.full_threshold`       | `--incremental-full-threshold <N>`     |
| `incremental.big_graph_threshold`  | `--incremental-big-graph-threshold <N>`|
| `dashboard.host`                   | `--dashboard-host <ip>`                |
| `dashboard.port`                   | `--dashboard-port <N>`                 |
| `dashboard.auto_open`              | `--dashboard-auto-open {true,false}`   |
| `git.commit_db`                    | `--git-commit-db {true,false}`         |
| `git.commit_embeddings`            | `--git-commit-embeddings {true,false}` |

`--embed-provider` is a value-enum. Typos (`olama` vs `ollama`) get
rejected by clap at parse time rather than written into the YAML.

## `--dry-run` (preview only)

Prints the resulting YAML to stdout. **No filesystem side effects** —
no `create_dir_all`, no `.gitignore` update, no canonicalisation.

```bash
understandable init --dry-run --preset cloud-full --embed-model bge-m3
```

The LLM-led wizard runs this first to show the user the plan before
committing.

## `--force` (merge with backup)

`init` refuses to overwrite an existing `understandable.yaml` by
default. With `--force` it:

1. Saves the previous file as `understandable.yaml.bak` (skip with
   `--no-backup`).
2. **Merges** the existing YAML with the preset and CLI overrides.
   Hand-typed fields you didn't pass on the command line survive.
3. Writes the new YAML.

Skip the merge with `--no-merge` for a clean rewrite (only the preset
+ CLI flags survive).

:::caution
`serde_yaml_ng` has no comment-preserving round-trip, so comments in
the previous file are dropped during the merge. `init` warns you and
the `.bak` lets you recover anything you cared about.
:::

## Auto-managed gitignore block

Unless you pass `--no-gitignore`, `init` writes (or rewrites) a
managed block in `<project>/.gitignore` between
`# >>> understandable >>>` and `# <<< understandable <<<` markers.

The block content depends on `git.commit_db`:

**`commit_db: true` (default, recommended):**

```gitignore
# >>> understandable >>>
# managed by `understandable init` — leave the DB tracked.
.understandable/intermediate/
.understandable/tmp/
# <<< understandable <<<
```

**`commit_db: false`:**

```gitignore
# >>> understandable >>>
# managed by `understandable init` — DB stays out of git.
.understandable/
# <<< understandable <<<
```

Re-running `init` updates the block in place. Hand-written entries
outside the markers are never touched.

## Russian / multilingual content

When the project ships docs or strings in non-English languages, pick
a multilingual embedding model. `bge-m3` on Ollama is the safe
default:

```bash
ollama pull bge-m3
understandable init --preset local-full --embed-model bge-m3
```

For OpenAI, `text-embedding-3-large` handles 100+ languages well; for
the local provider, `paraphrase-multilingual-mpnet-base-v2`.

## Examples

### Preview before writing

```bash
understandable init --dry-run --preset cloud-full
```

### CI-friendly, heuristic only

```bash
understandable init --preset minimal --no-gitignore
```

### Update existing config — bind dashboard to LAN

```bash
understandable init --force \
  --dashboard-host 0.0.0.0 --dashboard-auto-open false
```

### Air-gapped team, multilingual

```bash
understandable init --preset local-full --embed-model bge-m3 \
  --git-commit-db true
```

## See also

- [`analyze`](./analyze) — first place every YAML field is consumed.
- [`embed`](./embed) — provider selection at runtime.
- [Architecture](../architecture) — what the storage layout means.
