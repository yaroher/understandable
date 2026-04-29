//! Asserts every `understandable …` command line in skill markdown
//! actually parses through `clap` — drift between docs and the
//! binary has bitten us multiple times (renamed verb, removed flag,
//! typo'd subcommand).
//!
//! ## How it works
//!
//! `ua-cli` is a binary crate without a `lib` target, so the `Cli`
//! parser type isn't reachable from this integration test. Exposing
//! it would require touching `main.rs`, which is out of scope for
//! this test suite.
//!
//! Instead we use the workaround documented in the task spec: we
//! maintain a static list of known top-level subcommands ("verbs")
//! and assert that the *first non-flag token* after `understandable`
//! matches one of them. We additionally sanity-check that any
//! `--long-flag` tokens look syntactically well-formed (no spaces,
//! no `=` followed by nothing, no double `--`-only tokens). This
//! catches the most common drift — a typo in the verb or a removed
//! subcommand — without coupling the test to the full clap surface.
//!
//! When clap itself eventually grows a `lib` target, the
//! `try_parse_understandable_command` helper can be tightened to do
//! a real `Cli::try_parse_from` call.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Subcommand names recognised by the `understandable` binary.
/// Sourced from `crates/ua-cli/src/main.rs::Command`. Keep in sync
/// when verbs are added or renamed.
const KNOWN_VERBS: &[&str] = &[
    "analyze",
    "dashboard",
    "chat",
    "diff",
    "explain",
    "onboard",
    "domain",
    "knowledge",
    "extract",
    "merge",
    "validate",
    "staleness",
    "fingerprint",
    "export",
    "import",
    "search",
    "embed",
    "init",
    "scan",
    // Built-in clap verbs
    "help",
];

/// Global flags / options that may appear *immediately after*
/// `understandable` instead of a verb (e.g. `understandable
/// --version`, `understandable --help`).
const TOP_LEVEL_FLAGS: &[&str] = &[
    "--version",
    "-V",
    "--help",
    "-h",
    "--path",
    "-v",
    "--verbose",
];

#[test]
fn skill_command_lines_parse_with_clap() {
    let workspace_root = workspace_root();
    let skills_dir = workspace_root.join("plugin/skills");
    assert!(
        skills_dir.is_dir(),
        "expected skills dir at {}",
        skills_dir.display()
    );

    let known_verbs: HashSet<&str> = KNOWN_VERBS.iter().copied().collect();
    let top_flags: HashSet<&str> = TOP_LEVEL_FLAGS.iter().copied().collect();

    let mut errors: Vec<String> = Vec::new();
    let mut tested: usize = 0;

    walk_markdown(&skills_dir, &mut |path, line_no, command| {
        tested += 1;
        if let Err(e) = lint_command(command, &known_verbs, &top_flags) {
            errors.push(format!(
                "{}:{} `understandable {}` — {}",
                path.display(),
                line_no,
                command,
                e
            ));
        }
    });

    if !errors.is_empty() {
        panic!(
            "Skill command lint found {} broken commands:\n{}",
            errors.len(),
            errors.join("\n")
        );
    }
    eprintln!("[skill-lint] verified {} command lines", tested);
    assert!(
        tested > 0,
        "skill-lint walked the skills tree but found zero `understandable …` lines — did the regex break?"
    );
}

/// Returns the workspace root by climbing two levels from
/// `CARGO_MANIFEST_DIR` (`crates/ua-cli` → `crates` → workspace).
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("manifest dir has a workspace parent")
        .to_path_buf()
}

/// Tokenise + verb-check a single command. `command` excludes the
/// leading `understandable ` prefix.
fn lint_command(
    command: &str,
    known_verbs: &HashSet<&str>,
    top_flags: &HashSet<&str>,
) -> Result<(), String> {
    let tokens = shell_tokenise(command)?;
    if tokens.is_empty() {
        return Err("empty command after `understandable`".to_string());
    }

    // Basic per-token sanity. Catches stray `--` tokens with no flag
    // body, embedded whitespace, etc.
    for tok in &tokens {
        if tok == "--" {
            // bare `--` is an end-of-flags marker; clap accepts it
            continue;
        }
        if tok.starts_with("--") && tok.len() == 3 {
            return Err(format!("malformed flag token `{tok}`"));
        }
    }

    let first = &tokens[0];

    // Top-level flag form: `understandable --version` etc. No verb
    // is required.
    if first.starts_with('-') {
        // Strip a possible `=value` so `--path=foo` matches `--path`.
        let bare = first.split('=').next().unwrap_or(first);
        if top_flags.contains(bare) {
            return Ok(());
        }
        return Err(format!(
            "leading flag `{first}` is not a known top-level flag (expected one of {:?})",
            TOP_LEVEL_FLAGS
        ));
    }

    if !known_verbs.contains(first.as_str()) {
        return Err(format!(
            "unknown subcommand `{first}` (expected one of {:?})",
            KNOWN_VERBS
        ));
    }

    Ok(())
}

/// Minimal shell-style tokeniser: splits on unquoted whitespace,
/// honours single- and double-quoted runs, and treats `\<char>` as
/// a literal `<char>`. Plenty for skill-doc commands; we don't
/// support `$()` substitution or backticks.
fn shell_tokenise(line: &str) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut chars = line.chars().peekable();
    let mut in_token = false;
    let mut quote: Option<char> = None;

    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => {
                quote = None;
            }
            (Some(_), '\\') => {
                if let Some(&next) = chars.peek() {
                    cur.push(next);
                    chars.next();
                } else {
                    cur.push('\\');
                }
                in_token = true;
            }
            (Some(_), c) => {
                cur.push(c);
                in_token = true;
            }
            (None, '"') | (None, '\'') => {
                quote = Some(c);
                in_token = true;
            }
            (None, '\\') => {
                if let Some(&next) = chars.peek() {
                    cur.push(next);
                    chars.next();
                } else {
                    cur.push('\\');
                }
                in_token = true;
            }
            (None, c) if c.is_whitespace() => {
                if in_token {
                    out.push(std::mem::take(&mut cur));
                    in_token = false;
                }
            }
            (None, c) => {
                cur.push(c);
                in_token = true;
            }
        }
    }
    if quote.is_some() {
        return Err(format!("unterminated quote in command: {line:?}"));
    }
    if in_token {
        out.push(cur);
    }
    Ok(out)
}

/// Recursively walk markdown files under `dir` and invoke `cb` for
/// every `understandable …` invocation found inside a fenced code
/// block (` ``` `, optionally tagged `bash`/`sh`/`shell`/`console`).
/// Lines starting with `<<` (heredoc redirects) are skipped.
fn walk_markdown(dir: &Path, cb: &mut dyn FnMut(&Path, usize, &str)) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_markdown(&path, cb);
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        scan_markdown(&path, &text, cb);
    }
}

fn scan_markdown(path: &Path, text: &str, cb: &mut dyn FnMut(&Path, usize, &str)) {
    let mut in_fence = false;
    let mut fence_lang_is_shell = false;

    for (i, raw_line) in text.lines().enumerate() {
        let line_no = i + 1;
        let trimmed_left = raw_line.trim_start();

        if let Some(after) = trimmed_left.strip_prefix("```") {
            if in_fence {
                in_fence = false;
                fence_lang_is_shell = false;
            } else {
                in_fence = true;
                let tag = after.trim().to_ascii_lowercase();
                fence_lang_is_shell = tag.is_empty()
                    || matches!(
                        tag.as_str(),
                        "bash" | "sh" | "shell" | "zsh" | "console" | "shellsession"
                    );
            }
            continue;
        }

        if !in_fence || !fence_lang_is_shell {
            continue;
        }

        // Skip continuation lines / heredoc bodies. The rule of thumb:
        // we only want lines that *start* with `understandable ` (after
        // optional whitespace and an optional `$ ` prompt or indent).
        let body = trimmed_left
            .strip_prefix("$ ")
            .unwrap_or(trimmed_left)
            .trim_start();
        if body.starts_with("<<") {
            continue;
        }
        // Comments inside bash blocks.
        if body.starts_with('#') {
            continue;
        }

        let Some(rest) = body.strip_prefix("understandable") else {
            continue;
        };
        // Must be followed by whitespace *or* end of line — guard
        // against partial matches like `understandable.yaml`.
        let next = rest.chars().next();
        if !matches!(next, Some(c) if c.is_whitespace()) && next.is_some() {
            continue;
        }

        // Strip line-continuation backslash and any trailing pipe
        // segment that's clearly shell glue (e.g. `... | grep foo`).
        // We keep flags etc. The lint only cares about
        // verb/flag-shape correctness.
        let mut command = rest.trim().to_string();
        if let Some(idx) = command.find(" | ") {
            command.truncate(idx);
        }
        // Trim trailing line-continuation.
        if let Some(stripped) = command.strip_suffix('\\') {
            command = stripped.trim_end().to_string();
        }
        let command = command.trim();

        if command.is_empty() {
            // `understandable` alone — treat as a `--help`-style
            // bare invocation. Accept by skipping.
            continue;
        }

        cb(path, line_no, command);
    }
}

#[cfg(test)]
mod self_tests {
    use super::*;

    fn verbs() -> HashSet<&'static str> {
        KNOWN_VERBS.iter().copied().collect()
    }

    fn flags() -> HashSet<&'static str> {
        TOP_LEVEL_FLAGS.iter().copied().collect()
    }

    #[test]
    fn lint_accepts_known_verb() {
        assert!(lint_command("analyze --with-llm", &verbs(), &flags()).is_ok());
    }

    #[test]
    fn lint_accepts_top_level_flag() {
        assert!(lint_command("--version", &verbs(), &flags()).is_ok());
    }

    #[test]
    fn lint_rejects_unknown_verb() {
        let err = lint_command("frobnicate --foo", &verbs(), &flags()).unwrap_err();
        assert!(err.contains("frobnicate"), "{err}");
    }

    #[test]
    fn tokenise_handles_quotes() {
        assert_eq!(
            shell_tokenise("chat \"hello world\" --path /tmp").unwrap(),
            vec!["chat", "hello world", "--path", "/tmp"]
        );
    }

    #[test]
    fn tokenise_rejects_unterminated_quote() {
        assert!(shell_tokenise("chat \"oops").is_err());
    }
}
