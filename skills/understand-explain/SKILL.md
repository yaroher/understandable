---
name: understand-explain
description: Use when you need a deep-dive explanation of a specific file, function, or module in the codebase
argument-hint: [file-path|file-path:symbol]
---

# /understand-explain

Provide a thorough, in-depth explanation of a specific code component
using the persisted knowledge graph at `.understandable/graph.tar.zst`.

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

3. Parse the user's `$ARGUMENTS` into a target spec:
   - File-only: `src/auth/login.ts`
   - File + symbol: `src/auth/login.ts:verifyToken`

4. Run the explain subcommand. It resolves the target node, gathers
   the 1-hop subgraph, and identifies the architectural layer — then
   formats the result as an LLM prompt scaffold. The Rust builder is
   metadata-only (it does NOT read the source file or synthesise
   prose); the IDE LLM is responsible for the deep-dive explanation.

   ```bash
   understandable explain "$ARGUMENTS" --path "$PROJECT_ROOT"
   ```

5. The binary's stdout is **a context-prompt for the IDE LLM**, not a
   finished explanation. It contains:
   - The target node's id, type, summary, tags, and complexity
   - Internal structure (functions/classes contained, from `contains`
     edges)
   - Outgoing connections (imports, calls, depends_on)
   - Incoming connections (callers, importers, dependents)
   - The architectural layer and its description
   - A structured framing block that asks the IDE LLM to walk the
     reader through inputs, processing, and outputs

   You (the IDE LLM) read that scaffold, optionally open the actual
   file with `Read` for code grounding, and produce the explanation
   yourself. Quote node ids and layer names from the scaffold; do NOT
   echo the scaffold verbatim to the user.

6. If the explain command reports the node was not found, surface
   that to the user along with the binary's suggested similar paths.
   Do NOT fall back to `Grep`/`Read` against any JSON file — the
   persisted graph is a compressed binary store (`graph.tar.zst`).
