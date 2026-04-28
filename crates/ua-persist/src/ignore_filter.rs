//! `.gitignore` + `.understandignore` aware project file walker.

use std::path::{Path, PathBuf};

use ignore::{overrides::OverrideBuilder, WalkBuilder};

#[derive(Debug, Clone)]
pub struct IgnoreFilter {
    pub follow_links: bool,
    pub respect_gitignore: bool,
    pub respect_understandignore: bool,
    pub include_hidden: bool,
    /// Extra path prefixes layered on top of `.gitignore` and
    /// `.understandignore` — typically the `ignore.paths` block from
    /// `understandable.yaml`. Honoured natively by the `ignore` crate
    /// via [`OverrideBuilder`] so the walker never recurses into the
    /// excluded subtree (the previous post-filter approach paid the
    /// IO cost of walking it anyway).
    pub extra_ignore_paths: Vec<String>,
}

impl Default for IgnoreFilter {
    fn default() -> Self {
        Self {
            follow_links: false,
            respect_gitignore: true,
            respect_understandignore: true,
            include_hidden: false,
            extra_ignore_paths: Vec::new(),
        }
    }
}

/// Yield every project file path that survives the ignore filter.
pub fn walk_project(
    root: impl AsRef<Path>,
    filter: &IgnoreFilter,
) -> impl Iterator<Item = PathBuf> {
    let root_path = root.as_ref();
    let mut builder = WalkBuilder::new(root_path);
    builder
        .follow_links(filter.follow_links)
        .git_ignore(filter.respect_gitignore)
        .git_global(filter.respect_gitignore)
        .git_exclude(filter.respect_gitignore)
        .ignore(true)
        .hidden(!filter.include_hidden)
        .parents(true);
    if filter.respect_gitignore {
        // Respect `.gitignore` even outside an initialised repo.
        builder.add_custom_ignore_filename(".gitignore");
    }
    if filter.respect_understandignore {
        builder.add_custom_ignore_filename(".understandignore");
    }

    // Native plumbing for `IgnoreSettings.paths`: feed each entry into
    // an `OverrideBuilder` as a gitignore-style ignore pattern. The
    // override matcher's `add(glob)` defaults to whitelist semantics
    // (matched globs are *kept*), so each pattern is prefixed with `!`
    // to flip it back into an exclude. We register two flavours per
    // entry — the bare token (matches any file/dir with that basename
    // anywhere in the tree) and the `entry/` form (matches any
    // directory with that name and everything beneath it) — to cover
    // both `vendor` and `target/` style YAML inputs without forcing
    // the user to remember the trailing slash.
    if !filter.extra_ignore_paths.is_empty() {
        let mut ob = OverrideBuilder::new(root_path);
        for raw in &filter.extra_ignore_paths {
            let trimmed = raw.trim().trim_end_matches('/');
            if trimmed.is_empty() {
                continue;
            }
            // Best-effort: malformed globs are surfaced via
            // `tracing::warn!` rather than aborting the walk — the
            // caller would rather see *something* than nothing if a
            // single rule is bad.
            if let Err(e) = ob.add(&format!("!{trimmed}")) {
                tracing::warn!(pattern = %trimmed, error = %e, "ignore.paths: bad glob, skipping");
            }
            if let Err(e) = ob.add(&format!("!{trimmed}/**")) {
                tracing::warn!(pattern = %trimmed, error = %e, "ignore.paths: bad dir glob, skipping");
            }
        }
        match ob.build() {
            Ok(ov) => {
                builder.overrides(ov);
            }
            Err(e) => {
                tracing::warn!(error = %e, "ignore.paths: override build failed; extra paths ignored");
            }
        }
    }

    let walk = builder.build();
    walk.filter_map(Result::ok).filter_map(|d| {
        if d.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            Some(d.into_path())
        } else {
            None
        }
    })
}
