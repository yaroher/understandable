---
title: dashboard
sidebar_position: 2
---

# `understandable dashboard`

Boots the embedded axum server and serves the React UI. The dashboard
is bundled into the binary at compile time via `rust-embed` — there
is no separate frontend to install.

## Synopsis

```bash
understandable dashboard [--kind {codebase,domain,knowledge}] \
                         [--port <N>] [--host <ip>] \
                         [--open | --no-open]
```

## Flags

| Flag             | Default                       | Notes                                                                         |
|------------------|-------------------------------|-------------------------------------------------------------------------------|
| `--port <N>`     | `dashboard.port` / 5173       | Bind port.                                                                    |
| `--host <ip>`    | `dashboard.host` / 127.0.0.1  | Bind address. Use `0.0.0.0` for LAN access.                                   |
| `--open`         | —                             | Force-open a browser tab regardless of YAML.                                  |
| `--no-open`      | —                             | Force-don't-open. Mutually exclusive with `--open`.                           |
| `--kind <kind>`  | `codebase`                    | Which graph to serve: `codebase`, `domain`, `knowledge`.                      |

`--open` and `--no-open` together error out. With neither, the YAML
default (`dashboard.auto_open`, default `true`) wins.

## Multi-graph view

The same binary stores up to three independent graphs side by side:

- **`codebase`** — the default, written by `understandable analyze`.
  Files, symbols, calls, imports.
- **`domain`** — written by `understandable domain`. Domain / flow /
  step substrate derived from the codebase graph.
- **`knowledge`** — written by `understandable knowledge <wiki>`.
  Karpathy-wiki-style article/topic graph.

Each lives in its own archive (`graph.tar.zst`,
`graph.domain.tar.zst`, `graph.knowledge.tar.zst`). Run multiple
dashboards on different ports to view them side by side:

```bash
understandable dashboard --kind codebase --port 5173 &
understandable dashboard --kind domain   --port 5174 &
```

Picking a kind whose archive doesn't exist yet errors with a precise
pointer at which subcommand to run first.

## API endpoints

The axum server exposes a JSON API on the same port. The React app
consumes it; you can hit it directly from `curl` or build your own
client. See [Architecture](../architecture) for the full endpoint
list and storage format.

## Loopback by default

The server binds `127.0.0.1` unless you override. Nothing is exposed
to the network without an explicit opt-in.

To expose on the LAN (e.g. for a teammate to view your local graph):

```bash
understandable dashboard --host 0.0.0.0 --no-open
```

:::caution
The dashboard has no authentication. Don't bind `0.0.0.0` on an
untrusted network. For real multi-user deployments, terminate TLS +
auth on a reverse proxy and keep the dashboard on loopback.
:::

## Examples

### Default

```bash
understandable dashboard
# → http://127.0.0.1:5173/
```

### Custom port, headless

```bash
understandable dashboard --port 8080 --no-open
```

### Domain graph alongside the codebase graph

```bash
understandable domain
understandable dashboard --kind domain --port 5174
```

### LAN-accessible

```bash
understandable dashboard --host 0.0.0.0 --port 5173 --no-open
```

## See also

- [`analyze`](./analyze) — populate the codebase graph.
- [`embed`](./embed) — semantic search powered by the dashboard.
- [Architecture](../architecture) — endpoints and storage layout.
