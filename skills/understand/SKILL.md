---
name: understand
description: Analyze a codebase to produce an interactive knowledge graph for understanding architecture, components, and relationships
argument-hint: ["[path] [--full|--auto-update|--no-auto-update|--review]"]
---

# /understand

Analyze the current codebase and produce a `knowledge-graph.json` file in `.understandable/`. This file powers the interactive dashboard for exploring the project's architecture.

## Options

- `$ARGUMENTS` may contain:
  - `--full` — Force a full rebuild, ignoring any existing graph
  - `--auto-update` — Enable automatic graph updates on commit (writes `autoUpdate: true` to `.understandable/config.json`)
  - `--no-auto-update` — Disable automatic graph updates (writes `autoUpdate: false` to `.understandable/config.json`)
  - `--review` — Run full LLM graph-reviewer instead of inline deterministic validation
  - A directory path (e.g. `/path/to/repo` or `../other-project`) — Analyze the given directory instead of the current working directory

---

## Phase 0 — Pre-flight

Determine whether to run a full analysis or incremental update.

1. **Resolve `PROJECT_ROOT`:**
   - Parse `$ARGUMENTS` for a non-flag token (any argument that does not start with `--`). If found, treat it as the target directory path.
     - If the path is relative, resolve it against the current working directory.
     - Verify the resolved path exists and is a directory (run `test -d <path>`). If it does not exist or is not a directory, report an error to the user and **STOP**.
     - Set `PROJECT_ROOT` to the resolved absolute path.
   - If no directory path argument is found, set `PROJECT_ROOT` to the current working directory.
1.5. **Ensure the `understandable` binary is on `$PATH`.** Every phase
   below shells out to `understandable` (no Node.js / pnpm involved
   anymore — extraction, merging, validation and persistence all live
   in the Rust crate).

   ```bash
   if ! command -v understandable >/dev/null 2>&1; then
     echo "Error: the \`understandable\` binary is not on \$PATH."
     echo "Install it once:"
     echo "  cargo install --git https://github.com/yaroher/understandable understandable \\"
     echo "    --features all-langs,local-embeddings"
     exit 1
   fi
   ```

   If the user is missing `cargo`, point them at <https://rustup.rs>
   first. The full-features install is the recommended default — it
   bundles every tree-sitter grammar (~30 languages on top of the 11
   tier-1) plus the offline ONNX embedding runtime so `embed --provider
   local` works without an API key.

2. Get the current git commit hash:
   ```bash
   git rev-parse HEAD
   ```
3. Create the intermediate and temp output directories:
   ```bash
   mkdir -p $PROJECT_ROOT/.understandable/intermediate
   mkdir -p $PROJECT_ROOT/.understandable/tmp
   ```
3.5. **Auto-update configuration:**
   - If `--auto-update` is in `$ARGUMENTS`: write `{"autoUpdate": true}` to `$PROJECT_ROOT/.understandable/config.json`
   - If `--no-auto-update` is in `$ARGUMENTS`: write `{"autoUpdate": false}` to `$PROJECT_ROOT/.understandable/config.json`
   - These flags only set the config — analysis proceeds normally regardless.

4. **Check for subdomain knowledge graphs to merge:**
   List all `*knowledge-graph*.json` files in `$PROJECT_ROOT/.understandable/` **excluding** `knowledge-graph.json` itself (e.g. `frontend-knowledge-graph.json`, `backend-knowledge-graph.json`). If any subdomain graphs exist, invoke the merge subcommand. `--inputs` is repeatable and expects each subdomain graph as its own argument — passing a directory will not work. Use a shell loop to expand the glob:
   ```bash
   inputs=()
   for f in "$PROJECT_ROOT"/.understandable/*knowledge-graph*.json; do
     [ "$(basename "$f")" = "knowledge-graph.json" ] && continue
     inputs+=(--inputs "$f")
   done
   understandable merge --kind subdomain "${inputs[@]}" \
     --out "$PROJECT_ROOT/.understandable/graph.tar.zst"
   ```
   The script discovers subdomain graphs, loads the existing `knowledge-graph.json` as a base (if present), and merges everything into `knowledge-graph.json` (deduplicating nodes and edges). Report the merge summary to the user, then continue with the merged graph.

5. Check if `$PROJECT_ROOT/.understandable/knowledge-graph.json` exists. If it does, read it.
6. Check if `$PROJECT_ROOT/.understandable/meta.json` exists. If it does, read it to get `gitCommitHash`.
7. **Decision logic:**

   | Condition | Action |
   |---|---|
   | `--full` flag in `$ARGUMENTS` | Full analysis (all phases) |
   | No existing graph or meta | Full analysis (all phases) |
   | `--review` flag + existing graph + unchanged commit hash | Skip to Phase 6 (review-only — reuse existing assembled graph) |
   | Existing graph + unchanged commit hash | Ask the user: "The graph is up to date at this commit. Would you like to: **(a)** run a full rebuild (`--full`), **(b)** run the LLM graph reviewer (`--review`), or **(c)** do nothing?" Then follow their choice. If they pick (c), STOP. |
   | Existing graph + changed files | Incremental update (re-analyze changed files only) |

   **Review-only path:** Copy the existing `knowledge-graph.json` to `$PROJECT_ROOT/.understandable/intermediate/assembled-graph.json`, then jump directly to Phase 6 step 3.

   For incremental updates, get the changed file list:
   ```bash
   git diff <lastCommitHash>..HEAD --name-only
   ```
   If this returns no files, report "Graph is up to date" and STOP.

8. **Collect project context for subagent injection:**
   - Read `README.md` (or `README.rst`, `readme.md`) from `$PROJECT_ROOT` if it exists. Store as `$README_CONTENT` (first 3000 characters).
   - Read the primary package manifest (`package.json`, `pyproject.toml`, `Cargo.toml`, `go.mod`, `pom.xml`) if it exists. Store as `$MANIFEST_CONTENT`.
   - Capture the top-level directory tree:
     ```bash
     find $PROJECT_ROOT -maxdepth 2 -type f -not -path '*/node_modules/*' -not -path '*/.git/*' -not -path '*/dist/*' | head -100
     ```
     Store as `$DIR_TREE`.
   - Detect the project entry point by checking for common patterns (in order): `src/index.ts`, `src/main.ts`, `src/App.tsx`, `index.js`, `main.py`, `manage.py`, `app.py`, `wsgi.py`, `asgi.py`, `run.py`, `__main__.py`, `main.go`, `cmd/*/main.go`, `src/main.rs`, `src/lib.rs`, `src/main/java/**/Application.java`, `Program.cs`, `config.ru`, `index.php`. Store first match as `$ENTRY_POINT`.

---

## Phase 0.5 — Ignore Configuration

Set up and verify the `.understandignore` file before scanning. The
`understandable` walker honours `.gitignore` and `.understandignore`
natively, so this phase only needs to make sure a starter ignore file
exists and that the user has had a chance to review it.

1. Check if `$PROJECT_ROOT/.understandable/.understandignore` exists.
2. **If it does NOT exist**, run `understandable init` to scaffold the
   `.understandable/` directory and generate a starter ignore file
   seeded with the project's `.gitignore` plus sensible defaults:
   ```bash
   understandable init --path "$PROJECT_ROOT"
   ```
   If the user wants to pre-seed extra exclusion patterns inline,
   pass them as repeated `--ignore-path` flags:
   ```bash
   understandable init --path "$PROJECT_ROOT" \
     --ignore-path "tests/" \
     --ignore-path "fixtures/" \
     --ignore-path "*.snap"
   ```
   Report to the user:
   > Generated `.understandable/.understandignore` with suggested
   > exclusions based on your project structure. Please review it and
   > uncomment any patterns you'd like to exclude from analysis. When
   > ready, confirm to continue.
   - **Wait for user confirmation before proceeding.**
3. **If it already exists**, report:
   > Found `.understandable/.understandignore`. Review it if needed,
   > then confirm to continue.
   - **Wait for user confirmation before proceeding.**
4. After confirmation, proceed to Phase 1.

---

## Phase 1 — SCAN (Full analysis only)

Dispatch a subagent using the `project-scanner` agent definition (at `agents/project-scanner.md`). Append the following additional context:

> **Additional context from main session:**
>
> Project README (first 3000 chars):
> ```
> $README_CONTENT
> ```
>
> Package manifest:
> ```
> $MANIFEST_CONTENT
> ```
>
> Use this context to produce more accurate project name, description, and framework detection. The README and manifest are authoritative — prefer their information over heuristics.

Pass these parameters in the dispatch prompt:

> Scan this project directory to discover all project files (including non-code files like configs, docs, infrastructure), detect languages and frameworks.
> Project root: `$PROJECT_ROOT`
> Write output to: `$PROJECT_ROOT/.understandable/intermediate/scan-result.json`

After the subagent completes, read `$PROJECT_ROOT/.understandable/intermediate/scan-result.json` to get:
- Project name, description
- Languages, frameworks
- File list with line counts and `fileCategory` per file (`code`, `config`, `docs`, `infra`, `data`, `script`, `markup`)
- Complexity estimate
- Import map (`importMap`): pre-resolved project-internal imports per file (non-code files have empty arrays)

Store `importMap` in memory as `$IMPORT_MAP` for use in Phase 2 batch construction.
Store the file list as `$FILE_LIST` with `fileCategory` metadata for use in Phase 2 batch construction.

**Gate check:** If >100 files, inform the user and suggest scoping with a subdirectory argument. Proceed only if user confirms or add guidance that this may take a while.

If the scan result includes `filteredByIgnore > 0`, report:
> Excluded {filteredByIgnore} files via `.understandignore`.

---

## Phase 2 — ANALYZE

### Full analysis path

Batch the file list from Phase 1 into groups of **20-30 files each** (aim for ~25 files per batch for balanced sizes).

**Batching strategy for non-code files:**
- Group related non-code files together in the same batch when possible:
  - Dockerfile + docker-compose.yml + .dockerignore → same batch
  - SQL migration files → same batch (ordered by filename)
  - CI/CD config files (.github/workflows/*) → same batch
  - Documentation files (docs/*.md) → same batch
- This allows the file-analyzer to create cross-file edges (e.g., docker-compose `depends_on` Dockerfile)
- Non-code files can be mixed with code files in the same batch if batch sizes are small
- Each file's `fileCategory` from Phase 1 must be included in the batch file list

For each batch, dispatch a subagent using the `file-analyzer` agent definition (at `agents/file-analyzer.md`). Run up to **5 subagents concurrently** using parallel dispatch. Append the following additional context:

> **Additional context from main session:**
>
> Project: `<projectName>` — `<projectDescription>`
> Languages: `<languages from Phase 1>`

Before dispatching each batch, construct `batchImportData` from `$IMPORT_MAP`:
```json
batchImportData = {}
for each file in this batch:
  batchImportData[file.path] = $IMPORT_MAP[file.path] ?? []
```

Fill in batch-specific parameters below and dispatch:

> Analyze these files and produce GraphNode and GraphEdge objects.
> Project root: `$PROJECT_ROOT`
> Project: `<projectName>`
> Languages: `<languages>`
> Batch index: `<batchIndex>`
> Skill directory (for bundled scripts): `<SKILL_DIR>`
> Write output to: `$PROJECT_ROOT/.understandable/intermediate/batch-<batchIndex>.json`
>
> Pre-resolved import data for this batch (use this for all import edge creation — do NOT re-resolve imports from source):
> ```json
> <batchImportData JSON>
> ```
>
> Files to analyze in this batch:
> 1. `<path>` (<sizeLines> lines, fileCategory: `<fileCategory>`)
> 2. `<path>` (<sizeLines> lines, fileCategory: `<fileCategory>`)
> ...

After ALL batches complete, invoke the merge-and-normalize subcommand. `--inputs` is repeatable and takes individual file paths — pass each `batch-*.json` as its own `--inputs` argument:
```bash
inputs=()
for f in "$PROJECT_ROOT"/.understandable/intermediate/batch-*.json; do
  inputs+=(--inputs "$f")
done
understandable merge --kind file "${inputs[@]}" \
  --out "$PROJECT_ROOT/.understandable/graph.tar.zst"
```

This script reads all `batch-*.json` files from `$PROJECT_ROOT/.understandable/intermediate/`, then in one pass:
- Combines all nodes and edges across batches
- Normalizes node IDs (strips double prefixes, project-name prefixes, adds missing prefixes)
- Normalizes complexity values (`low`→`simple`, `medium`→`moderate`, `high`→`complex`, etc.)
- Rewrites edge references to match corrected node IDs
- Deduplicates nodes by ID (keeps last occurrence) and edges by `(source, target, type)`
- Drops dangling edges referencing missing nodes
- Logs all corrections and dropped items to stderr

Output: `$PROJECT_ROOT/.understandable/intermediate/assembled-graph.json`

Include the script's warnings in `$PHASE_WARNINGS` for the reviewer.

### Incremental update path

Use the changed files list from Phase 0. Batch and dispatch file-analyzer subagents using the same process as above (20-30 files per batch, up to 5 concurrent, with batchImportData constructed from $IMPORT_MAP), but only for changed files.

After batches complete:
1. Remove old nodes whose `filePath` matches any changed file from the existing graph
2. Remove old edges whose `source` or `target` references a removed node
3. Write the pruned existing nodes/edges as `batch-existing.json` in the intermediate directory
4. Run the same merge subcommand — it will combine `batch-existing.json` with the fresh `batch-*.json` files. As above, `--inputs` is per-file and repeatable:
   ```bash
   inputs=()
   for f in "$PROJECT_ROOT"/.understandable/intermediate/batch-*.json; do
     inputs+=(--inputs "$f")
   done
   understandable merge --kind file "${inputs[@]}" \
     --out "$PROJECT_ROOT/.understandable/graph.tar.zst"
   ```

---

## Phase 3 — ASSEMBLE REVIEW

Dispatch a subagent using the `assemble-reviewer` agent definition (at `agents/assemble-reviewer.md`).

Pass these parameters in the dispatch prompt:

> Review the assembled graph at `$PROJECT_ROOT/.understandable/intermediate/assembled-graph.json`.
> Project root: `$PROJECT_ROOT`
> Batch files are at: `$PROJECT_ROOT/.understandable/intermediate/batch-*.json`
> Write review output to: `$PROJECT_ROOT/.understandable/intermediate/assemble-review.json`
>
> **Merge script report:**
> ```
> <paste the full stderr output from understandable merge>
> ```
>
> **Import map for cross-batch edge verification:**
> ```json
> $IMPORT_MAP
> ```

After the subagent completes, read `$PROJECT_ROOT/.understandable/intermediate/assemble-review.json` and add any notes to `$PHASE_WARNINGS`.

---

## Phase 4 — ARCHITECTURE

**Build the combined prompt template:**
1. Use the `architecture-analyzer` agent definition (at `agents/architecture-analyzer.md`).
2. **Language context injection:** For each language detected in Phase 1 (e.g., `python`, `markdown`, `dockerfile`, `yaml`, `sql`, `terraform`, `graphql`, `protobuf`, `shell`, `html`, `css`), read the file at `./languages/<language-id>.md` (e.g., `./languages/python.md`, `./languages/dockerfile.md`) and append its content after the base template under a `## Language Context` header. If the file does not exist for a detected language, skip it silently and continue. These files are in the `languages/` subdirectory next to this SKILL.md file. **Include non-code language snippets** — they provide edge patterns and summary styles for non-code files.
3. **Framework addendum injection:** For each framework detected in Phase 1 (e.g., `Django`), read the file at `./frameworks/<framework-id-lowercase>.md` (e.g., `./frameworks/django.md`) and append its full content after the language context. If the file does not exist for a detected framework, skip it silently and continue. These files are in the `frameworks/` subdirectory next to this SKILL.md file.

Append the language/framework context and the following additional context to the agent's prompt:

> **Additional context from main session:**
>
> Frameworks detected: `<frameworks from Phase 1>`
>
> Directory tree (top 2 levels):
> ```
> $DIR_TREE
> ```
>
> Use the directory tree, language context, and framework addendums (appended above) to inform layer assignments. Directory structure is strong evidence for layer boundaries. Non-code files (config, docs, infrastructure, data) should be assigned to appropriate layers — see the prompt template for guidance.

Pass these parameters in the dispatch prompt:

> Analyze this codebase's structure to identify architectural layers.
> Project root: `$PROJECT_ROOT`
> Write output to: `$PROJECT_ROOT/.understandable/intermediate/layers.json`
> Project: `<projectName>` — `<projectDescription>`
>
> File nodes (all node types — includes code files, config, document, service, pipeline, table, schema, resource, endpoint):
> ```json
> [list of {id, type, name, filePath, summary, tags} for ALL file-level nodes — omit complexity, languageNotes]
> ```
>
> Import edges:
> ```json
> [list of edges with type "imports"]
> ```
>
> All edges (for cross-category analysis — includes configures, documents, deploys, triggers, etc.):
> ```json
> [list of ALL edges — include all edge types]
> ```

After the subagent completes, read `$PROJECT_ROOT/.understandable/intermediate/layers.json` and normalize it into a final `layers` array. Apply these steps **in order**:

1. **Unwrap envelope:** If the file contains `{ "layers": [...] }` instead of a plain array, extract the inner array. (The prompt requests a plain array, but LLMs may still produce an envelope.)
2. **Rename legacy fields:** If any layer object has a `nodes` field instead of `nodeIds`, rename `nodes` → `nodeIds`. If `nodes` entries are objects with an `id` field rather than plain strings, extract just the `id` values into `nodeIds`.
3. **Synthesize missing IDs:** If any layer is missing an `id`, generate one as `layer:<kebab-case-name>`.
4. **Convert file paths:** If `nodeIds` entries are raw file paths without a known prefix (`file:`, `config:`, `document:`, `service:`, `pipeline:`, `table:`, `schema:`, `resource:`, `endpoint:`), convert them to `file:<relative-path>`.
5. **Drop dangling refs:** Remove any `nodeIds` entries that do not exist in the merged node set.

Each element of the final `layers` array MUST have this shape:

```json
[
  {
    "id": "layer:<kebab-case-name>",
    "name": "<layer name>",
    "description": "<what belongs in this layer>",
    "nodeIds": ["file:src/App.tsx", "config:tsconfig.json", "document:README.md"]
  }
]
```

All four fields (`id`, `name`, `description`, `nodeIds`) are required.

**For incremental updates:** Always re-run architecture analysis on the full merged node set, since layer assignments may shift when files change.

**Context for incremental updates:** When re-running architecture analysis, also inject the previous layer definitions:

> Previous layer definitions (for naming consistency):
> ```json
> [previous layers from existing graph]
> ```
>
> Maintain the same layer names and IDs where possible. Only add/remove layers if the file structure has materially changed.

---

## Phase 5 — TOUR

Dispatch a subagent using the `tour-builder` agent definition (at `agents/tour-builder.md`). Append the following additional context:

> **Additional context from main session:**
>
> Project README (first 3000 chars):
> ```
> $README_CONTENT
> ```
>
> Project entry point: `$ENTRY_POINT`
>
> Use the README to align the tour narrative with the project's own documentation. Start the tour from the entry point if one was detected. The tour should tell the same story the README tells, but through the lens of actual code structure.

Pass these parameters in the dispatch prompt:

> Create a guided learning tour for this codebase.
> Project root: `$PROJECT_ROOT`
> Write output to: `$PROJECT_ROOT/.understandable/intermediate/tour.json`
> Project: `<projectName>` — `<projectDescription>`
> Languages: `<languages>`
>
> Nodes (all file-level nodes — includes code files, config, document, service, pipeline, table, schema, resource, endpoint):
> ```json
> [list of {id, name, filePath, summary, type} for ALL file-level nodes — do NOT include function or class nodes]
> ```
>
> Layers:
> ```json
> [list of {id, name, description} for each layer — omit nodeIds]
> ```
>
> Edges (all types — includes imports, calls, configures, documents, deploys, triggers, etc.):
> ```json
> [list of ALL edges — include all edge types for complete graph topology analysis]
> ```

After the subagent completes, read `$PROJECT_ROOT/.understandable/intermediate/tour.json` and normalize it into a final `tour` array. Apply these steps **in order**:

1. **Unwrap envelope:** If the file contains `{ "steps": [...] }` instead of a plain array, extract the inner array. (The prompt requests a plain array, but LLMs may still produce an envelope.)
2. **Rename legacy fields:** If any step has `nodesToInspect` instead of `nodeIds`, rename it → `nodeIds`. If any step has `whyItMatters` instead of `description`, rename it → `description`.
3. **Convert file paths:** If `nodeIds` entries are raw file paths without a known prefix (`file:`, `config:`, `document:`, `service:`, `pipeline:`, `table:`, `schema:`, `resource:`, `endpoint:`), convert them to `file:<relative-path>`.
4. **Drop dangling refs:** Remove any `nodeIds` entries that do not exist in the merged node set.
5. **Sort** by `order` before saving.

Each element of the final `tour` array MUST have this shape:

```json
[
  {
    "order": 1,
    "title": "Project Overview",
    "description": "Start with the README to understand the project's purpose and architecture.",
    "nodeIds": ["document:README.md"]
  },
  {
    "order": 2,
    "title": "Application Entry Point",
    "description": "This step explains how the frontend boots and mounts.",
    "nodeIds": ["file:src/main.tsx", "file:src/App.tsx"]
  }
]
```

Required fields: `order`, `title`, `description`, `nodeIds`. Preserve optional `languageLesson` when present.

---

## Phase 6 — REVIEW

Assemble the full KnowledgeGraph JSON object:

```json
{
  "version": "1.0.0",
  "project": {
    "name": "<projectName>",
    "languages": ["<languages>"],
    "frameworks": ["<frameworks>"],
    "description": "<projectDescription>",
    "analyzedAt": "<ISO 8601 timestamp>",
    "gitCommitHash": "<commit hash from Phase 0>"
  },
  "nodes": [<all nodes from assembled-graph.json after Phase 3 review>],
  "edges": [<all edges from assembled-graph.json after Phase 3 review>],
  "layers": [<layers from Phase 4>],
  "tour": [<steps from Phase 5>]
}
```

1. Before writing the assembled graph, validate that:
   - `layers` is an array of objects with these required fields: `id`, `name`, `description`, `nodeIds`
   - `tour` is an array of objects with these required fields: `order`, `title`, `description`, `nodeIds`
   - `tour[*].languageLesson` is allowed as an optional string field
   - Every `layers[*].nodeIds` entry exists in the merged node set
   - Every `tour[*].nodeIds` entry exists in the merged node set

   If validation fails, automatically normalize and rewrite the graph into this shape before saving. If the graph still fails final validation after the normalization pass, save it with warnings but mark dashboard auto-launch as skipped.

2. Write the assembled graph to `$PROJECT_ROOT/.understandable/intermediate/assembled-graph.json`.

3. **Check `$ARGUMENTS` for `--review` flag.** Then run the appropriate validation path:

---

#### Default path (no `--review`): deterministic validation

Run the binary's built-in validator. It performs every check the old
inline script did (schema, referential integrity, layer coverage,
duplicate ids, edge weight bounds, orphan nodes) in one pass and writes
the same `{issues, warnings, stats}` JSON shape.

```bash
understandable validate --json --path "$PROJECT_ROOT" \
  > "$PROJECT_ROOT/.understandable/intermediate/review.json"
```

The `--json` flag is required so the redirected file is the
machine-readable `{issues, warnings, stats}` document the rest of
this phase reads. Without it the binary prints human-readable text
and the file ends up unparseable.

If `understandable validate` exits non-zero, read its stderr, surface
the validation issues to the user, and continue with the
`review.json` it already wrote (a partial report is better than none).

---

#### `--review` path: full LLM reviewer

If `--review` IS in `$ARGUMENTS`, dispatch the LLM graph-reviewer subagent as follows:

Dispatch a subagent using the `graph-reviewer` agent definition (at `agents/graph-reviewer.md`). Append the following additional context:

> **Additional context from main session:**
>
> Phase 1 scan results (file inventory):
> ```json
> [list of {path, sizeLines} from scan-result.json]
> ```
>
> Phase warnings/errors accumulated during analysis:
> - [list any batch failures, skipped files, or warnings from Phases 2-5]
>
> Cross-validate: every file in the scan inventory should have a corresponding node in the graph (node types may vary: `file:`, `config:`, `document:`, `service:`, `pipeline:`, `table:`, `schema:`, `resource:`, `endpoint:`). Flag any missing files. Also flag any graph nodes whose `filePath` doesn't appear in the scan inventory.

Pass these parameters in the dispatch prompt:

> Validate the knowledge graph at `$PROJECT_ROOT/.understandable/intermediate/assembled-graph.json`.
> Project root: `$PROJECT_ROOT`
> Read the file and validate it for completeness and correctness.
> Write output to: `$PROJECT_ROOT/.understandable/intermediate/review.json`

---

4. Read `$PROJECT_ROOT/.understandable/intermediate/review.json`.

5. **If `issues` array is non-empty:**
   - Review the `issues` list
   - Apply automated fixes where possible:
     - Remove edges with dangling references
     - Fill missing required fields with sensible defaults (e.g., empty `tags` -> `["untagged"]`, empty `summary` -> `"No summary available"`)
     - Remove nodes with invalid types
   - Re-run the final graph validation after automated fixes
   - If critical issues remain after one fix attempt, save the graph anyway but include the warnings in the final report and mark dashboard auto-launch as skipped

6. **If `issues` array is empty:** Proceed to Phase 7.

---

## Phase 7 — SAVE

1. Write the final knowledge graph to `$PROJECT_ROOT/.understandable/knowledge-graph.json`.

2. Write metadata to `$PROJECT_ROOT/.understandable/meta.json`:
   ```json
   {
     "lastAnalyzedAt": "<ISO 8601 timestamp>",
     "gitCommitHash": "<commit hash>",
     "version": "1.0.0",
     "analyzedFiles": <number of files analyzed>
   }
   ```

2.5. **Generate structural fingerprints** for all analyzed files. This
   creates the baseline for future automatic incremental updates. The
   fingerprints (blake3 file hashes plus tree-sitter-extracted
   structural signatures) are persisted alongside the graph in
   `$PROJECT_ROOT/.understandable/graph.tar.zst`.

   Run the binary's fingerprint subcommand:
   ```bash
   understandable fingerprint --path "$PROJECT_ROOT"
   ```

   This uses the same tree-sitter analysis pipeline as the main
   fingerprint engine, ensuring the baseline matches the comparison
   logic used during auto-updates.

3. Clean up intermediate files:
   ```bash
   rm -rf $PROJECT_ROOT/.understandable/intermediate
   rm -rf $PROJECT_ROOT/.understandable/tmp
   ```

4. Report a summary to the user containing:
   - Project name and description
   - Files analyzed / total files (with breakdown by fileCategory: code, config, docs, infra, data, script, markup)
   - Nodes created (broken down by type: file, function, class, config, document, service, table, endpoint, pipeline, schema, resource)
   - Edges created (broken down by type)
   - Layers identified (with names)
   - Tour steps generated (count)
   - Any warnings from the reviewer
   - Path to the output file: `$PROJECT_ROOT/.understandable/knowledge-graph.json`

5. Only automatically launch the dashboard by invoking the `/understand-dashboard` skill if final graph validation passed after normalization/review fixes.
   If final validation did not pass, report that the graph was saved with warnings and dashboard launch was skipped.

---

## Error Handling

- If any subagent dispatch fails, retry **once** with the same prompt plus additional context about the failure.
- Track all warnings and errors from each phase in a `$PHASE_WARNINGS` list. When using `--review`, pass this list to the graph-reviewer in Phase 6. On the default path, include accumulated warnings in the Phase 7 final report.
- If it fails a second time, skip that phase and continue with partial results.
- ALWAYS save partial results — a partial graph is better than no graph.
- Report any skipped phases or errors in the final summary so the user knows what happened.
- NEVER silently drop errors. Every failure must be visible in the final report.

---

## Reference: KnowledgeGraph Schema

### Node Types (13 total)
| Type | Description | ID Convention |
|---|---|---|
| `file` | Source code file | `file:<relative-path>` |
| `function` | Function or method | `function:<relative-path>:<name>` |
| `class` | Class, interface, or type | `class:<relative-path>:<name>` |
| `module` | Logical module or package | `module:<name>` |
| `concept` | Abstract concept or pattern | `concept:<name>` |
| `config` | Configuration file (YAML, JSON, TOML, env) | `config:<relative-path>` |
| `document` | Documentation file (Markdown, RST, TXT) | `document:<relative-path>` |
| `service` | Deployable service definition (Dockerfile, K8s) | `service:<relative-path>` |
| `table` | Database table or migration | `table:<relative-path>:<table-name>` |
| `endpoint` | API endpoint or route definition | `endpoint:<relative-path>:<endpoint-name>` |
| `pipeline` | CI/CD pipeline configuration | `pipeline:<relative-path>` |
| `schema` | Schema definition (GraphQL, Protobuf, Prisma) | `schema:<relative-path>` |
| `resource` | Infrastructure resource (Terraform, CloudFormation) | `resource:<relative-path>` |

### Edge Types (26 total)
| Category | Types |
|---|---|
| Structural | `imports`, `exports`, `contains`, `inherits`, `implements` |
| Behavioral | `calls`, `subscribes`, `publishes`, `middleware` |
| Data flow | `reads_from`, `writes_to`, `transforms`, `validates` |
| Dependencies | `depends_on`, `tested_by`, `configures` |
| Semantic | `related`, `similar_to` |
| Infrastructure | `deploys`, `serves`, `provisions`, `triggers` |
| Schema/Data | `migrates`, `documents`, `routes`, `defines_schema` |

### Edge Weight Conventions
| Edge Type | Weight |
|---|---|
| `contains` | 1.0 |
| `inherits`, `implements` | 0.9 |
| `calls`, `exports`, `defines_schema` | 0.8 |
| `imports`, `deploys`, `migrates` | 0.7 |
| `depends_on`, `configures`, `triggers` | 0.6 |
| `tested_by`, `documents`, `provisions`, `serves`, `routes` | 0.5 |
| All others | 0.5 (default) |
