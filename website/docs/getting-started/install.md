---
title: Install
sidebar_position: 1
---

# Install

`understandable` ships as a single Rust binary. Three install paths;
pick the one that fits how you already manage tools.

## 1. One-line shell installer (recommended)

The fastest path. Downloads a prebuilt binary for your platform and
drops it on your `PATH`.

```bash
curl -fsSL https://raw.githubusercontent.com/yaroher/understandable/main/install.sh | sh
```

No Rust toolchain required. Works on Linux x86_64 / aarch64 and macOS
(Intel + Apple Silicon).

:::tip
Inside Claude Code, Cursor, or any IDE that loads the
`install-understandable` skill, just say **"install understandable"**
(or **"установи understandable"**). The skill walks the install,
verifies the binary, and runs `--version` for you.
:::

## 2. `cargo binstall` (no toolchain needed)

If you have [`cargo-binstall`][binstall] but don't want a Rust
toolchain on the box, this fetches a prebuilt artifact straight from
the GitHub release:

```bash
cargo binstall understandable
```

Same binary as the shell installer, same feature set as the published
release.

## 3. `cargo install` from source (max flexibility)

Build from `git` when you want to flip Cargo features, target an
unreleased branch, or trim the binary:

```bash
# Recommended: every feature on (~80 MB binary)
cargo install --git https://github.com/yaroher/understandable understandable \
  --features all-langs,local-embeddings

# Trimmed builds (skip what you know you won't need)
cargo install --git https://github.com/yaroher/understandable understandable
cargo install --git https://github.com/yaroher/understandable understandable --features all-langs
cargo install --git https://github.com/yaroher/understandable understandable --features local-embeddings
```

Feature matrix:

| Feature             | Adds                                                        | Cost            |
|---------------------|-------------------------------------------------------------|-----------------|
| (default)           | 11 tier-1 grammars + OpenAI/Ollama embeddings via HTTP      | ~40 MB          |
| `all-langs`         | + ~30 tier-2 grammars (Bash, Lua, Swift, Zig, …)            | +~25 MB         |
| `local-embeddings`  | + fastembed-rs ONNX runtime + tokenizers + hf-hub           | +~30 MB on disk |

`local-embeddings` downloads the ONNX model on first run (~120 MB
cached under `~/.cache/fastembed`).

## Per-platform notes

### Linux

Works out of the box on glibc-based distros (Fedora, Ubuntu, Arch,
Debian). For Alpine / musl, use option 3 with `--target
x86_64-unknown-linux-musl`.

### macOS

Both Intel and Apple Silicon are supported. If Gatekeeper objects to
the unsigned binary, run it once with `xattr -d com.apple.quarantine
$(which understandable)`.

### Windows

Native Windows builds work via option 3, but the test-and-tooling
story is best on **WSL2 (Ubuntu)**. We recommend installing inside
WSL — every example in these docs assumes a POSIX shell.

## Verifying the install

```bash
understandable --version
```

If you intend to use the local (offline) embeddings provider, also
verify the feature is compiled in:

```bash
understandable embed --help | grep -q "local" && echo "local feature ON" \
  || echo "local feature OFF"
```

`local feature OFF` plus a desire to use offline ONNX embeddings
means a re-install with `--features local-embeddings`.

## Next

Head to [Your First Graph](./first-graph) to scaffold a config and
build the first knowledge graph.

[binstall]: https://github.com/cargo-bins/cargo-binstall
