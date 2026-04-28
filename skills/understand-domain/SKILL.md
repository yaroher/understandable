---
description: Extract business domain knowledge from a codebase and generate an interactive domain flow graph backed by the understandable CLI.
argument-hint: "--full"
---

# /understand-domain

Extracts business domain knowledge — domains, business flows, and
process steps — from a codebase and produces an interactive horizontal
flow graph in the dashboard. The persisted domain graph is stored at
`.understandable/graph.domain.tar.zst` (sibling to `graph.tar.zst`).

## Prerequisites

The skill shells out to the `understandable` Rust binary. It must be
on `$PATH`.

## Instructions

### Phase 1 — Resolve target

1. Determine `PROJECT_ROOT` from the user's argument or the current
   working directory.
2. Note whether `--full` was passed in `$ARGUMENTS`. The binary uses
   the existing structural graph at `.understandable/graph.tar.zst` as
   context if it is present, falling back to a lightweight scan
   otherwise. `--full` forces a fresh scan even when the structural
   graph exists.

### Phase 2 — Build the domain graph

Run the domain subcommand. It performs the entire pipeline in one
call:
- detects entry points (HTTP routes, CLI commands, event handlers,
  cron jobs, exported handlers)
- groups them into domains and flows
- writes the persisted domain graph to
  `$PROJECT_ROOT/.understandable/graph.domain.tar.zst`
- emits a structured human-readable summary on stdout

```bash
understandable domain --path "$PROJECT_ROOT"
```

If `--full` was passed, append it:

```bash
understandable domain --path "$PROJECT_ROOT" --full
```

### Phase 3 — Inspect (optional)

If the user wants to see the underlying domain graph as JSON for
review, export it:

```bash
understandable export --kind domain --path "$PROJECT_ROOT" --pretty
```

Pipe that JSON to the user only when they explicitly ask for it.

### Phase 4 — Launch the dashboard

Auto-trigger the `/understand-dashboard` skill so the user can browse
the domain view. The dashboard detects `graph.domain.tar.zst` and
shows the domain layout by default.
