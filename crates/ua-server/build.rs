// build.rs for ua-server
//
// Ensures dashboard/dist/index.html exists before rust-embed tries to embed it.
// Priority:
//   1. Prebuilt dist already present → nothing to do.
//   2. pnpm available → run `pnpm install --frozen-lockfile && pnpm build`.
//   3. npm available  → run `npm install --legacy-peer-deps && npm run build`.
//   4. Neither found  → write a minimal stub so the crate compiles; the stub
//      tells the user how to get the real dashboard.
//
// This path is exercised only by `cargo install --git ...` or first-time local
// builds. CI and release workflows pre-build the dashboard explicitly and never
// reach the fallback logic.

use std::path::Path;
use std::process::Command;

fn main() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dashboard_dir = manifest_dir.join("../../dashboard");
    let dist_dir = dashboard_dir.join("dist");
    let index = dist_dir.join("index.html");

    // Re-run this script only when the built artifact changes.
    println!("cargo:rerun-if-changed={}", index.display());

    if index.exists() {
        println!("cargo:warning=ua-server: dashboard/dist already present; skipping build");
        return;
    }

    if try_build(
        &dashboard_dir,
        "pnpm",
        &["install", "--frozen-lockfile"],
        &["build"],
    ) {
        return;
    }
    if try_build(
        &dashboard_dir,
        "npm",
        &["install", "--legacy-peer-deps"],
        &["run", "build"],
    ) {
        return;
    }

    // Neither package manager found — write a stub so the binary at least
    // compiles. The stub page explains how to get the real dashboard.
    println!(
        "cargo:warning=ua-server: pnpm/npm not found; \
         writing stub dashboard/dist/index.html — \
         CLI subcommands work, but the web dashboard will show a placeholder"
    );
    std::fs::create_dir_all(&dist_dir).expect("failed to create dashboard/dist");
    std::fs::write(&index, STUB_HTML).expect("failed to write stub index.html");
}

/// Try to build the dashboard with `tool`. Returns true on success.
fn try_build(dir: &Path, tool: &str, install_args: &[&str], build_args: &[&str]) -> bool {
    if Command::new(tool).arg("--version").output().is_err() {
        return false;
    }

    println!("cargo:warning=ua-server: building dashboard with {tool}");

    let install_ok = Command::new(tool)
        .args(install_args)
        .current_dir(dir)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !install_ok {
        println!("cargo:warning=ua-server: {tool} install step failed; trying next option");
        return false;
    }

    let build_ok = Command::new(tool)
        .args(build_args)
        .current_dir(dir)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !build_ok {
        println!("cargo:warning=ua-server: {tool} build step failed; trying next option");
        return false;
    }

    println!("cargo:warning=ua-server: dashboard built successfully with {tool}");
    true
}

const STUB_HTML: &str = r#"<!doctype html>
<html><head><meta charset="utf-8"><title>understandable</title></head>
<body style="font-family:system-ui;padding:2rem;max-width:42rem;margin:auto">
  <h1>Dashboard not built</h1>
  <p>The interactive dashboard requires Node.js + pnpm to compile.</p>
  <p>Build it manually then reinstall:</p>
  <pre>cd dashboard &amp;&amp; pnpm install &amp;&amp; pnpm build
cd .. &amp;&amp; cargo install --path crates/ua-cli --force --features all-langs,local-embeddings</pre>
  <p>Or download a prebuilt binary from
  <a href="https://github.com/yaroher/understandable/releases/latest">the latest release</a>.</p>
  <p>The CLI (<code>understandable analyze</code>, <code>search</code>, <code>export</code>, etc.)
  works without the dashboard.</p>
</body></html>
"#;
