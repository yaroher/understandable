---
description: Launch the embedded `understandable` dashboard to visualise a project's knowledge graph.
argument-hint: "project-path"
---

# /understand-dashboard

Boot the local axum server bundled with the `understandable` binary
and point the user at the dashboard URL. The React UI is compiled into
the binary via `rust-embed`, so there is no Vite, pnpm, or
`packages/dashboard/` directory to find.

## Instructions

1. **Resolve `PROJECT_ROOT`.**
   * If `$ARGUMENTS` contains a path, treat it as the project
     directory. Resolve relative paths against the cwd.
   * Otherwise use the current working directory.

2. **Verify the analyzed graph exists.**
   ```bash
   test -f "$PROJECT_ROOT/.understandable/graph.tar.zst"
   ```
   If not, tell the user:
   ```
   No knowledge graph at $PROJECT_ROOT/.understandable/graph.tar.zst.
   Run /understand first to analyze the project.
   ```

3. **Verify the binary is on `$PATH`.**
   ```bash
   command -v understandable >/dev/null 2>&1 || {
     echo "Install once with: cargo install --git https://github.com/yaroher/understandable understandable --features all-langs,local-embeddings"
     exit 1
   }
   ```

4. **Start the server in the background.** Pass the project path
   explicitly so cwd doesn't matter. The binary reads
   `dashboard.host` / `dashboard.port` / `dashboard.auto_open` from
   `understandable.yaml` if present; CLI flags below override those.

   ```bash
   understandable dashboard --path "$PROJECT_ROOT"
   ```

   If the user asked for a different port (e.g. `5173` is busy), pass
   `--port <N>`. Pass `--host 0.0.0.0` only when they explicitly want
   LAN access — default is loopback.

5. **Report the URL.**
   ```
   Dashboard started at http://127.0.0.1:<port>/
   Project: $PROJECT_ROOT/.understandable/graph.tar.zst

   Press Ctrl+C in the terminal to stop it.
   ```
   No `?token=` parameter — the server only listens on the loopback
   interface so there is no auth.

## Notes

- The dashboard pulls graph data from the local REST API at
  `/api/graph`, `/api/search`, `/api/neighbors?id=…`, etc. There is no
  flat JSON file to hand around.
- For multi-graph projects, pass `--kind {codebase,domain,knowledge}`
  to choose which graph to serve. Default is `codebase`. Run separate
  `understandable dashboard --kind …` instances on different ports if
  you want to view several at once.
