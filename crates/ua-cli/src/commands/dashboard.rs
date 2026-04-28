//! `understandable dashboard` — boot the embedded axum server.
//!
//! `--kind` selects which graph slot becomes the dashboard's primary
//! view. Defaults to `codebase`; `domain` / `knowledge` require the
//! matching `understandable domain` / `understandable knowledge` run
//! to have been completed first so the per-kind archive exists on
//! disk.

use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;

use clap::{Args as ClapArgs, ValueEnum};
use ua_core::ProjectSettings;
use ua_persist::ProjectLayout;

// TODO: factor GraphKind into a shared util/ module after multi-agent wave settles.
/// Which graph slot a kind-aware command should target.
///
/// Mirrors the string vocabulary used by `ProjectLayout::graph_archive_for`
/// so the conversion to `&str` at call sites stays a one-liner.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
#[clap(rename_all = "lowercase")]
pub enum GraphKind {
    Codebase,
    Domain,
    Knowledge,
}

impl GraphKind {
    pub fn as_str(self) -> &'static str {
        match self {
            GraphKind::Codebase => "codebase",
            GraphKind::Domain => "domain",
            GraphKind::Knowledge => "knowledge",
        }
    }
}

impl fmt::Display for GraphKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<GraphKind> for &'static str {
    fn from(k: GraphKind) -> &'static str {
        k.as_str()
    }
}

#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Port to bind. Falls back to `dashboard.port` in
    /// `understandable.yaml` (default 5173).
    #[arg(long)]
    pub port: Option<u16>,
    /// Bind address. Falls back to `dashboard.host` (default 127.0.0.1).
    #[arg(long)]
    pub host: Option<IpAddr>,
    /// Force-open a browser tab regardless of `dashboard.auto_open` in
    /// `understandable.yaml`. Mutually exclusive with `--no-open`.
    #[arg(long)]
    pub open: bool,
    /// Force-don't-open a browser tab regardless of
    /// `dashboard.auto_open` in `understandable.yaml`. Mutually
    /// exclusive with `--open`.
    #[arg(long)]
    pub no_open: bool,
    /// Which graph slot to serve. Defaults to `codebase`. Reads
    /// `<storage-dir>/<db-name>.<kind>.tar.zst` (or just
    /// `<db-name>.tar.zst` for `codebase`).
    #[arg(long, value_enum, default_value_t = GraphKind::Codebase)]
    pub kind: GraphKind,
}

/// Resolve the open/no-open flags + YAML default into a single bool.
///
/// Truth table:
///   neither flag → `settings.dashboard.auto_open` (YAML).
///   `--open`     → always open.
///   `--no-open`  → never open.
///   both         → error (caller bails).
fn resolve_auto_open(open: bool, no_open: bool, yaml_default: bool) -> anyhow::Result<bool> {
    match (open, no_open) {
        (true, true) => anyhow::bail!("`--open` and `--no-open` are mutually exclusive"),
        (true, false) => Ok(true),
        (false, true) => Ok(false),
        (false, false) => Ok(yaml_default),
    }
}

pub async fn run(args: Args, project: &Path) -> anyhow::Result<()> {
    let settings = ProjectSettings::load_or_default(project)?;
    let host = args
        .host
        .unwrap_or_else(|| settings.dashboard.host.parse().unwrap_or(IpAddr::from([127, 0, 0, 1])));
    let port = args.port.unwrap_or(settings.dashboard.port);
    let auto_open = resolve_auto_open(args.open, args.no_open, settings.dashboard.auto_open)?;
    let addr = SocketAddr::new(host, port);
    let url = format!("http://{addr}/");
    let kind: &str = args.kind.into();

    // Verify the chosen archive exists before we even bind a port —
    // saves the user from a half-booted dashboard with no data, and
    // gives them a precise pointer at which subcommand to run first.
    let layout = ProjectLayout::for_project(project);
    let archive = layout.graph_archive_for(kind);
    if kind != "codebase" && !archive.exists() {
        anyhow::bail!(
            "no archive at {} — run `understandable {}` first",
            archive.display(),
            kind,
        );
    }

    if auto_open {
        if let Err(e) = open_browser(&url) {
            tracing::warn!(?e, "failed to open browser; continue manually at {url}");
        }
    }
    println!("dashboard ready at {url} (kind={kind})  (Ctrl-C to stop)");
    ua_server::serve_kind(project, addr, kind).await
}

/// Spawn the platform "open this URL" helper after validating the URL.
///
/// Hardening rationale: the URL we receive here is built from settings
/// the user can write to, and shelling out with an arbitrary string
/// would let `xdg-open` interpret weird schemes (`file://`, `javascript:`,
/// custom handlers) or accidentally treat a flag-looking path as an
/// option. We therefore parse the URL with `url::Url` and reject any
/// scheme outside the http(s) allowlist.
///
/// We also reap the spawned `Child`: detached processes left behind
/// would otherwise become zombies on Linux until the parent exits.
fn open_browser(url: &str) -> std::io::Result<()> {
    let parsed = url::Url::parse(url).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("invalid dashboard URL '{url}': {e}"),
        )
    })?;
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("refusing to open URL with scheme '{scheme}'"),
        ));
    }

    #[cfg(target_os = "linux")]
    let prog = "xdg-open";
    #[cfg(target_os = "macos")]
    let prog = "open";
    #[cfg(target_os = "windows")]
    let prog = "explorer";

    let child = std::process::Command::new(prog)
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;

    // Reap the child in a background tokio task so it doesn't end up
    // as a zombie. We deliberately move the `Child` into the task and
    // ignore its exit status — the user only cares whether the spawn
    // itself succeeded.
    tokio::spawn(async move {
        let mut child = child;
        // `Child::wait` is blocking; offload to a blocking thread so
        // we don't park a tokio worker.
        let _ = tokio::task::spawn_blocking(move || child.wait()).await;
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{open_browser, resolve_auto_open, Args, GraphKind};
    use clap::Parser;

    /// Wrapper so we can drive `Args` through clap directly in tests.
    #[derive(Parser, Debug)]
    struct TestCli {
        #[command(flatten)]
        args: Args,
    }

    #[test]
    fn rejects_non_http_scheme() {
        let err = open_browser("file:///etc/passwd").unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn rejects_garbage_url() {
        let err = open_browser("not a url").unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn auto_open_truth_table() {
        // neither flag → YAML wins
        assert!(resolve_auto_open(false, false, true).unwrap());
        assert!(!resolve_auto_open(false, false, false).unwrap());
        // --open → always on
        assert!(resolve_auto_open(true, false, false).unwrap());
        // --no-open → always off
        assert!(!resolve_auto_open(false, true, true).unwrap());
    }

    #[test]
    fn args_open_and_no_open_conflict_errors() {
        // The flags themselves both parse; the conflict surfaces when
        // we resolve them. (Keeping clap-level validation off lets the
        // error live next to the truth-table comment above.)
        let cli = TestCli::try_parse_from(["dashboard", "--open", "--no-open"]).unwrap();
        let err = resolve_auto_open(cli.args.open, cli.args.no_open, true).unwrap_err();
        assert!(
            err.to_string().contains("mutually exclusive"),
            "expected mutually-exclusive error, got: {err}",
        );
    }

    #[test]
    fn kind_typo_rejected() {
        let res = TestCli::try_parse_from(["dashboard", "--kind", "codebse"]);
        assert!(res.is_err(), "clap should reject `--kind codebse`");
    }

    #[test]
    fn kind_default_is_codebase() {
        let cli = TestCli::try_parse_from(["dashboard"]).unwrap();
        assert_eq!(cli.args.kind, GraphKind::Codebase);
    }
}
