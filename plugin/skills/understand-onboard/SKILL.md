---
description: Use when you need to generate an onboarding guide for new team members joining a project
---

# /understand-onboard

Generate a comprehensive onboarding guide from the persisted knowledge
graph at `.understandable/graph.tar.zst`.

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

3. Run the onboard subcommand. It walks the persisted graph, picks
   out the project metadata, layers, tour, file-level nodes, and
   complexity hotspots, and synthesises a structured onboarding guide:

   ```bash
   understandable onboard --path "$PROJECT_ROOT"
   ```

4. Pipe the binary's stdout straight back to the user. The output is
   already organised into:
   - **Project Overview** — name, languages, frameworks, description
   - **Architecture Layers** — each layer's name, description, and
     key files
   - **Key Concepts** — patterns and design decisions surfaced from
     node summaries and tags
   - **Guided Tour** — the step-by-step walkthrough
   - **File Map** — what each key file does, organised by layer
   - **Complexity Hotspots** — files with the highest complexity for
     careful review

5. Offer to save the guide to `docs/ONBOARDING.md`. If the user
   agrees, write the binary's stdout there verbatim and suggest they
   commit it. Do NOT grep or parse `graph.tar.zst` manually — it is a
   compressed binary store.
