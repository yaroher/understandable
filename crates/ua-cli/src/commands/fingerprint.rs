//! `understandable fingerprint` — recompute file fingerprints and store
//! them in the persisted graph.

use std::path::Path;

use clap::Args as ClapArgs;
use ua_extract::{default_registry, LanguageRegistry};
use ua_persist::{blake3_file, walk_project, Fingerprint, IgnoreFilter, ProjectLayout, Storage};

#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Print the count of fingerprints, then exit (don't update the DB).
    #[arg(long)]
    pub dry_run: bool,
}

pub async fn run(args: Args, project_path: &Path) -> anyhow::Result<()> {
    let filter = IgnoreFilter::default();
    let lang_registry = LanguageRegistry::default_registry();
    let plugin_registry = default_registry();
    let mut prints = Vec::new();
    for path in walk_project(project_path, &filter) {
        let rel = path
            .strip_prefix(project_path)
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        let hash = match blake3_file(&path) {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(path = %rel, error = %e, "skipping unreadable file");
                continue;
            }
        };
        let modified_at = ua_persist::fingerprint::modtime_secs(&path);
        // Compute the structural hash inline so the same scan that
        // produces the byte-level hash also captures the AST shape;
        // skipping the second read avoids a doubled IO bill on large
        // repos. `structural_hash_of` returns `None` for unsupported
        // languages and parser failures — leave the field empty in
        // those cases so the change classifier falls back to its
        // regex heuristics.
        let structural_hash = lang_registry
            .for_path(&path)
            .map(|cfg| cfg.id.clone())
            .and_then(|lang| {
                std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|content| plugin_registry.structural_hash_of(&lang, &rel, &content))
            });
        prints.push(Fingerprint {
            path: rel,
            hash,
            modified_at,
            structural_hash,
        });
    }

    if args.dry_run {
        println!("would write {} fingerprints", prints.len());
        return Ok(());
    }

    let layout = ProjectLayout::for_project(project_path);
    layout.ensure_exists()?;
    let storage = Storage::open(&layout).await?;
    storage.write_fingerprints(&prints).await?;
    storage.save(&layout).await?;
    println!("wrote {} fingerprints", prints.len());
    Ok(())
}
