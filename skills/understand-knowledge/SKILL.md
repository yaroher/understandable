---
name: understand-knowledge
description: Analyze a Karpathy-pattern LLM wiki knowledge base and generate an interactive knowledge graph with entity extraction, implicit relationships, and topic clustering.
argument-hint: [wiki-directory]
---

# /understand-knowledge

Analyzes a Karpathy-pattern LLM wiki — a three-layer knowledge base with raw sources, wiki markdown, and a schema file — and produces an interactive knowledge graph dashboard.

## What It Detects

The **Karpathy LLM wiki pattern** (see https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f):
- **Raw sources** — immutable source documents (articles, papers, data files)
- **Wiki** — LLM-generated markdown files with wikilinks (`[[target]]` syntax)
- **Schema** — CLAUDE.md, AGENTS.md, or similar configuration file
- **index.md** — content catalog organized by categories
- **log.md** — chronological operation log

Detection signals: has `index.md` + multiple `.md` files with wikilinks. May have `raw/` directory and schema file.

## Instructions

### Phase 1 — Build the wiki knowledge graph

1. Determine `TARGET_DIR`:
   - If the user provided a path argument, use that.
   - Otherwise, use the current working directory.

2. Walk the wiki directory to confirm the Karpathy pattern is present.
   At minimum the directory should contain an `index.md` plus other
   markdown files. A `raw/` source directory and a schema file
   (CLAUDE.md / AGENTS.md) are optional but supported.

3. Run the knowledge subcommand with the wiki root. The binary builds
   the full knowledge graph in one go — wikilink extraction,
   index.md category resolution, source-node creation, layer/tour
   synthesis, and persistence to `.understandable/graph.knowledge.tar.zst`
   all happen inside this single invocation.

   ```bash
   understandable knowledge --path "$TARGET_DIR" "$TARGET_DIR"
   ```

   If the wiki root differs from the project root (e.g. `wiki/`
   nested inside a larger repo), pass the wiki root as the positional
   argument and the project root as `--path`:

   ```bash
   understandable knowledge --path "$PROJECT_ROOT" "$WIKI_ROOT"
   ```

   The binary refuses to run if the directory does not look like a
   Karpathy wiki. If it errors, tell the user it does not appear to
   be a Karpathy-pattern wiki and explain what was expected
   (`index.md` plus other markdown files with `[[wikilinks]]`).

4. Pipe the binary's stdout to the user. It already announces:
   - article / source / topic / wikilink counts (with unresolved
     wikilinks flagged)
   - the categories found from `index.md`
   - layer and tour summaries

### Phase 2 — Inspect (optional)

If the user wants to see the underlying knowledge graph as JSON for
review, export it:

```bash
understandable export --kind knowledge --path "$TARGET_DIR" --pretty
```

Pipe that JSON to the user only when they explicitly ask for it.

### Phase 3 — Launch the dashboard

Auto-trigger the `/understand-dashboard` skill so the user can browse
the wiki view:

```
/understand-dashboard $TARGET_DIR
```

The dashboard detects `graph.knowledge.tar.zst` and uses a
force-directed layout instead of hierarchical dagre.

## Notes

- The `understandable knowledge` subcommand handles ALL deterministic
  extraction (wikilinks, headings, frontmatter, categories from
  `index.md`) plus the implicit-claim inference internally — there is
  no separate scan-manifest, batched article-analyzer dispatch, or
  merge step exposed at the skill level.
- Categories and taxonomy come from `index.md` section headings, NOT
  from filename prefixes. The Karpathy spec is intentionally abstract
  about naming conventions.
- Source nodes from `raw/` are lightweight (filename + size only) —
  the binary does not parse PDFs or other binary files.
