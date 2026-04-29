# Auto-Update Knowledge Graph (Internal — Hook-Triggered)

Incrementally update the knowledge graph using deterministic structural
fingerprinting to minimise token usage. This prompt is triggered
automatically by the post-commit hook when `autoUpdate` is enabled.
**Not** a user-facing skill.

**Key principle:** spend zero LLM tokens when changes are cosmetic
(formatting, internal logic). Only invoke LLM agents when structural
changes (new/removed functions, classes, imports, exports) are
detected.

---

## Phase 0 — Pre-flight (zero token cost)

1. Set `PROJECT_ROOT` to the current working directory.

2. Check that `$PROJECT_ROOT/.understandable/graph.tar.zst` exists.
   If not, report `No knowledge graph found. Run /understand first.` and
   **STOP**.

3. Run `understandable validate --path $PROJECT_ROOT`.
   If it reports `graph valid`, capture the graph stats; otherwise STOP
   and surface the validation issues to the user.

4. Get the current HEAD and the graph's recorded commit hash:
   ```bash
   understandable staleness --path "$PROJECT_ROOT" --json
   ```
   The command returns a JSON document `{ "stale", "current_commit",
   "graph_commit", "drift_count" }` and exits with `0` (fresh), `1`
   (stale), `2` (no graph), or `3` (error). When invoked from the
   SessionStart / PostToolUse hook the prompt is triggered on exit-code
   `1` only, so `stale` is expected to be `true`. Capture
   `current_commit` and `graph_commit` from the JSON.

5. Get the changed files since the persisted graph's commit:
   ```bash
   git diff --name-only "$GRAPH_COMMIT".."$CURRENT_COMMIT"
   ```
   (`$GRAPH_COMMIT` and `$CURRENT_COMMIT` come from step 4's JSON.) If
   `git diff` fails — typically because `$GRAPH_COMMIT` is unreachable
   in the current history (rebased / squashed / shallow clone) — fall
   back to `git diff --name-only HEAD~1..HEAD` and treat every file as
   potentially structural in Phase 1.

   If no files changed and `--force` is not in `$ARGUMENTS`, report
   `Knowledge graph is already up to date.` and **STOP**.

6. Filter to source files only — extensions matching the languages
   reported by `understandable export | jq '.project.languages'`.
   If no source files changed, report
   `Only non-source files changed. Nothing to do.` and **STOP**.

---

## Phase 1 — Structural fingerprint check (zero LLM tokens)

This phase delegates the deterministic fingerprint comparison to the
`understandable` binary. It costs no LLM tokens.

```bash
understandable analyze --incremental --path $PROJECT_ROOT --plan-only \
  > $PROJECT_ROOT/.understandable/intermediate/change-analysis.json
```

The plan JSON has shape:

```json
{
  "action": "SKIP | PARTIAL_UPDATE | ARCHITECTURE_UPDATE | FULL_UPDATE",
  "reason": "1 file has structural changes (new function added)",
  "filesToReanalyze": ["src/new-feature.ts"],
  "rerunArchitecture": false,
  "rerunTour": false,
  "fileChanges": [
    { "filePath": "src/utils.ts",       "changeLevel": "COSMETIC",   "details": ["internal logic changed"] },
    { "filePath": "src/new-feature.ts", "changeLevel": "STRUCTURAL", "details": ["new function: handleRequest"] }
  ]
}
```

**Decision gate:**

| Action                | Do this                                                            |
|-----------------------|--------------------------------------------------------------------|
| `SKIP`                | Run `understandable fingerprint --path $PROJECT_ROOT` and **STOP**. |
| `FULL_UPDATE`         | Report scope, recommend `understandable analyze --full`. **STOP.** |
| `PARTIAL_UPDATE`      | Proceed to Phase 2 with `filesToReanalyze`.                        |
| `ARCHITECTURE_UPDATE` | Proceed to Phase 2 with `filesToReanalyze` + flag arch re-run.     |

---

## Phase 2 — Targeted re-analysis (minimal LLM cost)

Only re-analyse files with structural changes. This is the only phase
that costs LLM tokens. The persisted graph at
`.understandable/graph.tar.zst` is updated in place — there is no
export-to-JSON / import-from-JSON round-trip.

1. Batch the files from `filesToReanalyze` (one batch if ≤10 files,
   otherwise groups of 5–10).

2. Per batch, dispatch the `file-analyzer` agent (defined at
   `agents/file-analyzer.md`) with this header:

   > **Additional context from the main session:**
   >
   > Project: `<projectName>` — `<projectDescription>`
   > Frameworks: `<frameworks>`
   > Languages: `<languages>`
   >
   > **IMPORTANT:** This is an incremental update. Only the listed files
   > have structural changes. Analyse them but do not invent nodes for
   > files outside the batch.

   The agent shells out to `understandable extract --batch ... --out ...`
   for the deterministic part, just like a full `/understand` run, and
   writes its `batch-<N>.json` file under
   `$PROJECT_ROOT/.understandable/intermediate/`.

3. Fold the batch results into the persisted graph in place. The
   `merge --kind file` subcommand removes the prior nodes/edges for
   each touched file, adds the fresh ones from the batches, and
   deduplicates everything against the existing store:

   ```bash
   understandable merge --kind file \
     --inputs $PROJECT_ROOT/.understandable/intermediate \
     --out   $PROJECT_ROOT/.understandable/graph.tar.zst
   ```

   The merger handles deletion of files that no longer exist in
   `filesToReanalyze` automatically — supply the deleted-files list to
   the same intermediate directory as `batch-deleted.json` if the
   plan from Phase 1 included any.

---

## Phase 3 — Conditional architecture / tour + save

### 3a. Architecture update (only if `rerunArchitecture === true`)

Dispatch the `architecture-analyzer` agent against the merged node set.
Include the previous layers in the prompt so the agent maintains naming
consistency.

### 3b. Lite layer update (if `rerunArchitecture === false`)

- New files: append to the most likely existing layer based on directory match.
- Deleted files: remove their ids from each layer's `nodeIds`.
- Drop empty layers.

### 3c. Lite validation

Run `understandable validate --path "$PROJECT_ROOT"` after the merge.
The binary's validator catches dangling edges, duplicate ids, and bad
weights.

### 3d. Save

The merge in Phase 2 step 3 already wrote the updated graph to
`.understandable/graph.tar.zst` in place. There is no separate save
step.

1. Refresh fingerprints so the next incremental run has an accurate
   baseline:
   ```bash
   understandable fingerprint --path "$PROJECT_ROOT"
   ```

2. Clean up intermediates:
   ```bash
   rm -rf "$PROJECT_ROOT/.understandable/intermediate"
   ```

3. Report a summary:
   - Files checked: N (total changed)
   - Structural changes: N files
   - Cosmetic-only changes: N files (skipped)
   - Nodes updated: N
   - Action: PARTIAL_UPDATE / ARCHITECTURE_UPDATE
   - Output: `$PROJECT_ROOT/.understandable/graph.tar.zst`

---

## Error handling

- If `understandable analyze --incremental --plan-only` fails, fall back
  to treating every changed file as STRUCTURAL.
- If the fingerprints table is empty, treat every changed file as
  STRUCTURAL and regenerate after the update.
- If a subagent dispatch fails, retry once. On a second failure, save
  partial results and report the error.
- Always save partial results — a partially updated graph is better
  than no update.

---

## Notes

- The hook reuses `file-analyzer` and `architecture-analyzer` agents
  unchanged — no incremental-only agent prompts.
- Phase 1's fingerprint comparison uses the `understandable` binary's
  blake3 file hashes plus tree-sitter-extracted structural signatures
  stored in the `fingerprints` table inside `graph.tar.zst`.
