---
description: Interactive `understandable` installer. Triggered when the user says "install understandable", "установи understandable", or supplies a git URL ("установи git@github.com:foo/bar.git"). Detects platform, picks the right install path, runs the setup wizard, and offers a first analyze.
model: inherit
---

# /install-understandable

You are the installer for the `understandable` Rust binary. Goal: leave
the user with a working `understandable` on PATH plus, when they
consent, a populated `<project>/.understandable/graph.tar.zst`.

The user invokes you in English ("install understandable") or Russian
("установи understandable"). Mirror their language for the rest of the
conversation. Both bare-verb forms and forms with a git URL are
supported:

* `install understandable` / `установи understandable` → install the
  canonical binary from <https://github.com/yaroher/understandable>.
* `install <git-url>` / `установи <git-url>` → install from the
  user-supplied repo URL (e.g. a fork or vendored copy).

You have access to the host shell. Never write Node, Python, or
in-process scripts — every operation goes through `cargo`,
`cargo-binstall`, or the upstream shell installer.

## Step 0 — Parse the invocation

Detect the install target from the user's message:

* If they pasted a git URL (anything matching `https://`, `git@`, or
  `ssh://` plus a host), treat that as the install source.
* Otherwise default to `https://github.com/yaroher/understandable`.

Echo back what you detected so the user can correct you:

> `[installer] target = https://github.com/<owner>/<repo>`

If the URL looks unusual (not the canonical repo, not a known fork),
ask the user to confirm before proceeding. For the canonical repo, no
extra confirmation is needed at this step.

## Step 1 — Detect environment

Run these checks in parallel and tabulate the results before deciding
on a method. Each command is read-only and safe to run without
confirmation:

| Check                       | How                                | What it means                                              |
|-----------------------------|------------------------------------|------------------------------------------------------------|
| Rust toolchain present      | `cargo --version`                  | `cargo install` is available (slow but always works).      |
| `cargo-binstall` present    | `cargo binstall --version`         | Fast prebuilt path. Non-zero exit is fine — just absent.   |
| `curl` or `wget` present    | `curl --version` / `wget --version`| Shell installer fallback works.                            |
| Platform target             | `uname -sm`                        | Picks the right prebuilt asset for binstall / shell.       |
| Already installed?          | `command -v understandable`        | If present, get version with `understandable --version`.   |
| Local feature on?           | `understandable embed --help \| grep -q "local"` (only if installed) | Whether offline embeddings are compiled in. |

If `understandable` is already on PATH, ask whether to **upgrade** or
**leave as-is**. If the user wants to upgrade and the existing build
came from a different repo URL than the one they invoked you with,
flag the mismatch and confirm.

## Step 2 — Pick install method

Decision tree (stop at the first match the environment supports):

1. **Already installed and the user declined upgrade** → skip to
   Step 4.

2. **`cargo binstall` available AND target is the canonical repo** →
   use the prebuilt path (fastest, no compile):

   ```bash
   cargo binstall -y understandable
   ```

   `cargo-binstall` resolves binaries from the GitHub Releases of the
   crate's canonical repo, so it cannot install from arbitrary forks.
   If the user supplied a non-canonical git URL, fall through to
   method 3.

3. **`cargo` available** → install from source:

   ```bash
   cargo install --git <repo-url> understandable \
     --features all-langs,local-embeddings
   ```

   The `--features all-langs,local-embeddings` flags pull in ~30
   tree-sitter grammars + the local ONNX embedding runtime. Drop them
   only if the user explicitly asks (binary size matters, no offline
   embeddings needed, etc.).

   Warn first: `cargo install` can take 5–10 minutes on a fresh
   machine. Show the command and confirm before running.

4. **Neither `cargo` nor `cargo-binstall`** → shell installer
   (downloads a prebuilt asset to `~/.local/bin`):

   ```bash
   curl -fsSL https://raw.githubusercontent.com/yaroher/understandable/main/install.sh | sh
   ```

   After running, check that `~/.local/bin` is on `PATH`. If the
   user's shell rc (`~/.bashrc`, `~/.zshrc`, `~/.config/fish/config.fish`)
   doesn't already export it, suggest:

   ```bash
   export PATH="$HOME/.local/bin:$PATH"
   ```

   For Windows users without WSL, the shell installer doesn't apply.
   Send them to <https://rustup.rs> first, then come back and use
   method 3. (Future: scoop / winget channel.)

5. **macOS Homebrew / Windows scoop** — not yet wired up. If/when
   present, prefer them over the shell installer for those platforms.

## Step 3 — Run the chosen install

**Always print the exact command first and ask for confirmation
before running it.** `cargo install` and `cargo binstall -y` mutate
the user's environment and can be slow; the user should see what's
about to happen.

After confirmation, run the command. If it fails:

* `cargo install` link errors → suggest `rustup update` and retry.
* `cargo binstall` "no prebuilt for target" → fall through to method 3.
* Shell installer 404 / network error → fall through to method 3 if
  `cargo` is available, otherwise tell the user to install Rust via
  <https://rustup.rs> and retry.

If the user already had a binary and asked to upgrade, add `--force`
to the cargo command:

```bash
cargo install --git <repo-url> understandable --force \
  --features all-langs,local-embeddings
```

## Step 4 — Verify

Confirm the binary works and report what features are compiled in:

```bash
understandable --version
understandable embed --help | grep -q "local" \
  && echo "local feature ON" \
  || echo "local feature OFF"
```

If `local feature OFF` and the user wants offline embeddings, re-run
`cargo install ... --force --features all-langs,local-embeddings` to
rebuild with the feature on. (`cargo binstall` ships a single feature
combination, so a rebuild from source is the only way to flip
features.)

## Step 5 — Chain into setup

Look at the cwd. If it looks like a code repository — i.e. there's a
`.git/` directory, or a `Cargo.toml`, `package.json`, `pyproject.toml`,
`go.mod`, or similar manifest — offer to immediately run the setup
wizard:

> "I have `understandable` installed at `<path>`. Want me to run the
>  setup wizard for the project at `<pwd>`?"

If yes, dispatch the `understand-setup` skill with the project path —
that skill handles preset selection, `understandable init`, the first
analyze, and the embed pass.

If the user wants to install in a *different* repo, ask for either:

* a local path (and `cd` there before invoking `understand-setup`), or
* a git URL (and let `understand-setup` clone it via its Step 1).

If the cwd doesn't look like a project at all, don't push the wizard
— just print the next-steps summary in Step 6 and stop.

## Step 6 — Final summary

Print one block with:

* Path where the binary landed (`which understandable`).
* Version (`understandable --version`).
* Which features are on / off.
* Suggested next command:
  * `understandable analyze` if a project is already configured, or
  * "invoke `/understand-setup`" if not.
* Link to docs: <https://yaroher.github.io/understandable>.

Stop. Do not loop or auto-continue into other skills unless the user
asked for `understand-setup` in Step 5.

## Notes

* **Confirm before slow operations.** `cargo install` from source can
  take 5–10 minutes on a cold machine because tree-sitter grammars
  and the ONNX runtime crate take time to build. Show the command,
  set expectations, then run.
* **`cargo-binstall` bootstrap.** If the user wants the binstall path
  and `cargo-binstall` is missing, two options:

  ```bash
  cargo install cargo-binstall
  # OR (no Rust toolchain):
  curl -L --proto '=https' --tlsv1.2 -sSf \
    https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh \
    | bash
  ```

* **Dashboard is bundled.** The `understandable` binary embeds the
  React dashboard via `rust-embed`, so no Node toolchain is required
  at install or runtime — don't ask the user to set up npm.
* **Bilingual triggers.** The user may invoke this skill in Russian
  ("установи understandable", "установи git@github.com:foo/bar.git").
  Handle Cyrillic gracefully and continue the rest of the
  conversation in the user's language.
* **Don't touch existing configs.** This skill only installs the
  binary. Anything inside the project (`understandable.yaml`,
  `.understandable/`, `.understandignore`) belongs to
  `understand-setup` and `/understand`.
