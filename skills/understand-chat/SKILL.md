---
name: understand-chat
description: Use when you need to ask questions about a codebase or understand code using a knowledge graph
argument-hint: [query]
---

# /understand-chat

Answer questions about this codebase by querying the persisted
knowledge graph at `.understandable/graph.tar.zst` via the
`understandable` CLI.

## Prerequisites

The skill shells out to the `understandable` Rust binary. It must be
on `$PATH` and the project must already have a graph (run
`/understand` first if `.understandable/graph.tar.zst` is missing).

## Instructions

1. Resolve `PROJECT_ROOT`:
   - If the user supplied a directory argument, use that.
   - Otherwise, use the current working directory.

2. Verify the graph exists:
   ```bash
   test -f "$PROJECT_ROOT/.understandable/graph.tar.zst"
   ```
   If it does not exist, tell the user to run `/understand` first and
   STOP.

3. Run the chat subcommand with the user's query (`$ARGUMENTS`). The
   binary handles semantic search over node names, summaries, tags,
   and 1-hop edge expansion against the persisted graph — no manual
   JSON grepping required.

   ```bash
   understandable chat "$ARGUMENTS" --path "$PROJECT_ROOT" --limit 15
   ```

4. The binary's stdout is **a context-prompt scaffold for the IDE
   LLM**, not a finished natural-language answer. It contains:
   - the matched nodes (id, name, summary, tags, layer)
   - the 1-hop subgraph around each match (incoming and outgoing
     edges)
   - the architectural layer(s) the matches belong to
   - a structured "Question / Context / Instructions" framing the IDE
     LLM can complete

   You (the IDE LLM) are expected to read that scaffold and synthesise
   the actual answer for the user. Treat the binary's output as
   retrieval context — quote relevant ids, layer names, and edge
   relationships from it, but write the prose yourself. Do NOT just
   echo the scaffold to the user.

5. If the query returned zero matches, surface the binary's "no
   matches" message and suggest related search terms it printed.
   Do NOT fall back to `Grep`/`Read` against any JSON file — the
   persisted graph is a compressed binary store (`graph.tar.zst`).

6. If the user asks a follow-up that needs deeper context, re-run
   `understandable chat` with a refined query rather than parsing the
   previous output.
