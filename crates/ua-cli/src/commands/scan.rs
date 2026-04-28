//! `understandable scan` — bootstrap an `.understandignore`.
//!
//! The first concrete subcommand: derive a project-local
//! `.understandignore` from the existing `.gitignore` plus a managed
//! block of "things you almost certainly never want analysed" defaults
//! (binary build dirs, big blobs, lockfiles, IDE droppings, …). The
//! managed block is fenced by `# >>> understandable defaults >>>` /
//! `# <<< understandable defaults <<<` markers so a future re-run can
//! tell user-authored content from the bootstrap defaults.

use std::path::Path;

use clap::Args as ClapArgs;

const HEADER: &str = "# >>> understandable defaults >>>";
const FOOTER: &str = "# <<< understandable defaults <<<";

/// Standard high-noise paths / extensions we always want excluded from
/// graph extraction — emitted into the managed block of
/// `.understandignore` whenever the user runs `scan --gen-ignore`.
///
/// Lines beginning with `#` are written through verbatim as comments
/// so the bootstrap output is self-documenting.
const DEFAULTS: &[&str] = &[
    "# vcs / agent state",
    ".git/",
    ".understandable/",
    "# build & dependency dirs",
    "node_modules/",
    "target/",
    "dist/",
    "build/",
    "out/",
    ".next/",
    ".venv/",
    "__pycache__/",
    "# lockfiles — flip these off in monorepos where lockfiles encode",
    "# real architectural intent (workspace deps, pinned toolchains).",
    "*.lock",
    "# images / media",
    "*.png",
    "*.jpg",
    "*.jpeg",
    "*.webp",
    "*.gif",
    "# documents / archives",
    "*.pdf",
    "*.zip",
    "*.tar",
    "*.tar.gz",
    "*.tar.zst",
    "# binary blobs / model weights",
    "*.bin",
    "*.onnx",
    "*.safetensors",
    "*.exe",
    "*.dll",
    "*.so",
    "*.dylib",
    "# IDE / OS noise",
    ".idea/",
    ".vscode/",
    ".DS_Store",
];

#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Generate `<project>/.understandignore` from `.gitignore` + defaults.
    #[arg(long)]
    pub gen_ignore: bool,
    /// Overwrite an existing `.understandignore` if `--gen-ignore` is set.
    #[arg(long)]
    pub force: bool,
    /// Print the result to stdout instead of writing the file.
    #[arg(long)]
    pub dry_run: bool,
}

pub async fn run(args: Args, project: &Path) -> anyhow::Result<()> {
    if !args.gen_ignore {
        anyhow::bail!("scan currently supports --gen-ignore only");
    }

    let target = project.join(".understandignore");
    if target.exists() && !args.force && !args.dry_run {
        anyhow::bail!(
            ".understandignore already exists at {} — re-run with --force to overwrite \
             (or --dry-run to preview)",
            target.display()
        );
    }

    let gitignore_path = project.join(".gitignore");
    let gitignore_lines = read_gitignore_lines(&gitignore_path)?;
    let rendered = render(&gitignore_lines);

    if args.dry_run {
        // Print to stdout without trailing newline duplication — `rendered`
        // already ends in `\n`.
        print!("{rendered}");
        return Ok(());
    }

    std::fs::write(&target, &rendered)?;
    println!("wrote {}", target.display());
    Ok(())
}

/// Read `.gitignore` if present and strip blank / comment-only lines so
/// the user-content section of the rendered file mirrors the *intent*
/// of the gitignore rather than its formatting (the defaults block
/// already supplies its own comments).
fn read_gitignore_lines(path: &Path) -> anyhow::Result<Vec<String>> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(s
            .lines()
            .filter(|line| {
                let t = line.trim();
                !t.is_empty() && !t.starts_with('#')
            })
            .map(|l| l.to_string())
            .collect()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

/// Compose the final `.understandignore` body: gitignore-derived lines
/// (if any) up top, then the managed defaults block bracketed by
/// header/footer markers. Always ends with `\n`.
fn render(gitignore_lines: &[String]) -> String {
    let mut out = String::new();
    if !gitignore_lines.is_empty() {
        out.push_str("# from .gitignore\n");
        for line in gitignore_lines {
            out.push_str(line);
            out.push('\n');
        }
        out.push('\n');
    }
    out.push_str(HEADER);
    out.push('\n');
    for line in DEFAULTS {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str(FOOTER);
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cheap RAII tempdir without pulling in the `tempfile` crate as a
    /// dev-dep on `ua-cli`. Mirrors the helper in `commands::analyze`.
    struct ScratchDir {
        path: std::path::PathBuf,
    }

    impl ScratchDir {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0);
            let path = std::env::temp_dir().join(format!(
                "ua-cli-scan-test-{tag}-{}-{nanos}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &std::path::Path {
            &self.path
        }
    }

    impl Drop for ScratchDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn run_blocking(args: Args, project: &Path) -> anyhow::Result<()> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(run(args, project))
    }

    #[test]
    fn gen_ignore_writes_the_defaults_block() {
        let dir = ScratchDir::new("defaults");
        std::fs::write(
            dir.path().join(".gitignore"),
            "# user comment\nnode_modules/\n\nfoo.log\n",
        )
        .unwrap();

        run_blocking(
            Args {
                gen_ignore: true,
                force: false,
                dry_run: false,
            },
            dir.path(),
        )
        .unwrap();

        let body = std::fs::read_to_string(dir.path().join(".understandignore")).unwrap();
        assert!(body.contains(HEADER), "header marker present: {body}");
        assert!(body.contains(FOOTER), "footer marker present");
        // Sample of the defaults block:
        assert!(body.contains("target/"));
        assert!(body.contains(".DS_Store"));
        assert!(body.contains("*.safetensors"));
        // Gitignore content carried through, comments dropped:
        assert!(body.contains("node_modules/"));
        assert!(body.contains("foo.log"));
        assert!(!body.contains("# user comment"));
    }

    #[test]
    fn existing_file_refuses_without_force() {
        let dir = ScratchDir::new("noforce");
        std::fs::write(dir.path().join(".understandignore"), "old contents\n").unwrap();

        let err = run_blocking(
            Args {
                gen_ignore: true,
                force: false,
                dry_run: false,
            },
            dir.path(),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains(".understandignore already exists"),
            "unexpected error: {err}"
        );
        // Sanity: file untouched.
        let body = std::fs::read_to_string(dir.path().join(".understandignore")).unwrap();
        assert_eq!(body, "old contents\n");

        // With `--force` it overwrites cleanly.
        run_blocking(
            Args {
                gen_ignore: true,
                force: true,
                dry_run: false,
            },
            dir.path(),
        )
        .unwrap();
        let body = std::fs::read_to_string(dir.path().join(".understandignore")).unwrap();
        assert!(body.contains(HEADER));
    }

    #[test]
    fn dry_run_prints_and_does_not_write() {
        let dir = ScratchDir::new("dryrun");
        run_blocking(
            Args {
                gen_ignore: true,
                force: false,
                dry_run: true,
            },
            dir.path(),
        )
        .unwrap();
        assert!(
            !dir.path().join(".understandignore").exists(),
            "dry_run must not create the file"
        );
    }

    #[test]
    fn render_without_gitignore_only_emits_defaults_block() {
        let out = render(&[]);
        assert!(out.starts_with(HEADER));
        assert!(out.trim_end().ends_with(FOOTER));
        assert!(!out.contains("# from .gitignore"));
    }

    #[test]
    fn read_gitignore_strips_comments_and_blanks() {
        let dir = ScratchDir::new("strip");
        std::fs::write(
            dir.path().join(".gitignore"),
            "\n# comment\n  \nfoo\nbar/\n",
        )
        .unwrap();
        let lines = read_gitignore_lines(&dir.path().join(".gitignore")).unwrap();
        assert_eq!(lines, vec!["foo".to_string(), "bar/".to_string()]);
    }

    #[test]
    fn read_gitignore_missing_is_empty() {
        let dir = ScratchDir::new("missing");
        let lines = read_gitignore_lines(&dir.path().join(".gitignore")).unwrap();
        assert!(lines.is_empty());
    }

    #[test]
    fn run_without_gen_ignore_bails() {
        let dir = ScratchDir::new("nogen");
        let err = run_blocking(
            Args {
                gen_ignore: false,
                force: false,
                dry_run: false,
            },
            dir.path(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("--gen-ignore"));
    }
}
