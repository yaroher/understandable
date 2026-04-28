---
name: understand-diff
description: Use when you need to analyze git diffs or pull requests to understand what changed, affected components, and risks
---

# /understand-diff

Analyze the current code changes against the persisted knowledge graph
at `.understandable/graph.tar.zst` via the `understandable` CLI.

## Prerequisites

The skill shells out to the `understandable` Rust binary. It must be
on `$PATH` and the project must already have a graph (run
`/understand` first if `.understandable/graph.tar.zst` is missing).

## Instructions

1. Resolve `PROJECT_ROOT` from the user's argument or default to the
   current working directory.

2. Verify the graph exists:
   ```bash
   test -f "$PROJECT_ROOT/.understandable/graph.tar.zst"
   ```
   If it does not exist, tell the user to run `/understand` first and
   STOP.

3. Run the diff subcommand. It auto-detects the git context — staged
   changes, working-tree changes, or `<base>..HEAD` for a feature
   branch — and emits a structured impact report:

   ```bash
   understandable diff --path "$PROJECT_ROOT"
   ```

   - To restrict the analysis to an explicit file list, pass `--files`
     followed by one or more space-separated paths (e.g.
     `understandable diff --path "$PROJECT_ROOT" --files src/auth.ts src/api.ts`).
     Each path is its own argument — no commas, no quoting needed for
     plain unix paths.
   - To target a specific PR or base branch, set the git context
     beforehand (`git fetch origin pr/123/head:pr-123 && git checkout pr-123`)
     and re-run the same command.

4. Pipe the binary's stdout straight back to the user. The output
   already contains:
   - **Changed Components** — file/function nodes whose source files
     were modified (with summaries and complexity)
   - **Affected Components** — 1-hop upstream callers and downstream
     dependencies
   - **Affected Layers** — the architectural layers touched
   - **Risk Assessment** — derived from node complexity and blast
     radius across layers
   - A diff overlay JSON at
     `$PROJECT_ROOT/.understandable/diff-overlay.json` that the
     dashboard reads automatically. `understandable diff` writes this
     file atomically (tmp + rename) on every run unless `--no-write` is
     passed. The dashboard's `/api/diff` endpoint reads it from the
     same path; without it the endpoint returns 204 No Content.

5. After printing the report, tell the user they can run
   `/understandable:understand-dashboard` to see the diff overlay
   visualised. Do NOT grep or parse `graph.tar.zst` manually — it is a
   compressed binary store.

   To suppress the overlay write (e.g. one-off CLI inspection), pass
   `--no-write`:

   ```bash
   understandable diff --path "$PROJECT_ROOT" --no-write
   ```
