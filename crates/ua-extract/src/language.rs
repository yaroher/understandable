//! Language registry: maps file extensions and basenames to a language id.
//!
//! Mirrors the lookup behaviour of `LanguageRegistry` in the TS port —
//! filename-based match is preferred (handles `Dockerfile`, `Makefile`,
//! `docker-compose.yml`, etc.) and falls back to extension lookup.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Stable language id used everywhere (graph node tags, framework lookup,
/// extractor dispatch). Values are the canonical lowercase form.
pub type LanguageId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LanguageConfig {
    pub id: LanguageId,
    pub display_name: String,
    pub extensions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filenames: Vec<String>,
    /// Programming concepts this language exposes — used by the
    /// language-lesson generator. Free-form labels.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub concepts: Vec<String>,
    /// Whether the registry has a tree-sitter grammar wired up for this id.
    #[serde(default)]
    pub has_tree_sitter: bool,
}

#[derive(Debug, Default, Clone)]
pub struct LanguageRegistry {
    by_id: HashMap<String, LanguageConfig>,
    by_extension: HashMap<String, String>,
    by_filename: HashMap<String, String>,
}

impl LanguageRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, cfg: LanguageConfig) {
        for ext in &cfg.extensions {
            let key = if ext.starts_with('.') {
                ext.to_lowercase()
            } else {
                format!(".{}", ext.to_lowercase())
            };
            self.by_extension.insert(key, cfg.id.clone());
        }
        for fname in &cfg.filenames {
            self.by_filename
                .insert(fname.to_lowercase(), cfg.id.clone());
        }
        self.by_id.insert(cfg.id.clone(), cfg);
    }

    pub fn get(&self, id: &str) -> Option<&LanguageConfig> {
        self.by_id.get(id)
    }

    pub fn for_path(&self, path: &Path) -> Option<&LanguageConfig> {
        if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
            if let Some(id) = self.by_filename.get(&name.to_lowercase()) {
                return self.by_id.get(id);
            }
        }
        if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
            let key = format!(".{}", ext.to_lowercase());
            if let Some(id) = self.by_extension.get(&key) {
                return self.by_id.get(id);
            }
        }
        None
    }

    pub fn all(&self) -> impl Iterator<Item = &LanguageConfig> {
        self.by_id.values()
    }

    /// Pre-populated registry with all tier-1 + most common tier-2 languages.
    pub fn default_registry() -> Self {
        let mut r = Self::new();
        for cfg in builtin_configs() {
            r.register(cfg);
        }
        r
    }
}

fn lc(
    id: &str,
    display: &str,
    exts: &[&str],
    filenames: &[&str],
    concepts: &[&str],
    tree_sitter: bool,
) -> LanguageConfig {
    LanguageConfig {
        id: id.to_string(),
        display_name: display.to_string(),
        extensions: exts.iter().map(|s| s.to_string()).collect(),
        filenames: filenames.iter().map(|s| s.to_string()).collect(),
        concepts: concepts.iter().map(|s| s.to_string()).collect(),
        has_tree_sitter: tree_sitter,
    }
}

fn builtin_configs() -> Vec<LanguageConfig> {
    vec![
        lc(
            "typescript",
            "TypeScript",
            &["ts", "tsx", "mts", "cts"],
            &[],
            &["generics", "type-narrowing", "decorators", "modules"],
            true,
        ),
        lc(
            "javascript",
            "JavaScript",
            &["js", "jsx", "mjs", "cjs"],
            &[],
            &["closures", "prototypes", "promises", "modules"],
            true,
        ),
        lc(
            "python",
            "Python",
            &["py", "pyi", "pyw"],
            &[],
            &[
                "decorators",
                "context-managers",
                "generators",
                "duck-typing",
            ],
            true,
        ),
        lc(
            "go",
            "Go",
            &["go"],
            &[],
            &["interfaces", "goroutines", "channels", "error-values"],
            true,
        ),
        lc(
            "rust",
            "Rust",
            &["rs"],
            &[],
            &["ownership", "traits", "lifetimes", "pattern-matching"],
            true,
        ),
        lc(
            "java",
            "Java",
            &["java"],
            &[],
            &["generics", "annotations", "interfaces", "streams"],
            true,
        ),
        lc(
            "ruby",
            "Ruby",
            &["rb"],
            &["Rakefile", "Gemfile"],
            &["blocks", "metaprogramming", "modules"],
            true,
        ),
        lc(
            "php",
            "PHP",
            &["php"],
            &[],
            &["traits", "interfaces", "namespaces"],
            true,
        ),
        lc(
            "c",
            "C",
            &["c", "h"],
            &[],
            &["pointers", "manual-memory", "macros"],
            true,
        ),
        lc(
            "cpp",
            "C++",
            &["cpp", "cc", "cxx", "hpp", "hxx", "hh"],
            &[],
            &["templates", "raii", "operator-overloading"],
            true,
        ),
        lc(
            "csharp",
            "C#",
            &["cs"],
            &[],
            &["linq", "async-await", "generics", "properties"],
            true,
        ),
        // tier-2 / metadata-only
        lc(
            "env",
            "dotenv",
            &["env"],
            &[
                ".env",
                ".env.local",
                ".env.example",
                ".env.production",
                ".env.development",
            ],
            &[],
            false,
        ),
        lc("ini", "INI", &["ini", "cfg"], &[], &[], false),
        lc("yaml", "YAML", &["yaml", "yml"], &[], &[], false),
        lc("json", "JSON", &["json"], &[], &[], false),
        lc("toml", "TOML", &["toml"], &[], &[], false),
        lc(
            "dockerfile",
            "Dockerfile",
            &["dockerfile"],
            &["Dockerfile", "Containerfile"],
            &[],
            false,
        ),
        lc(
            "makefile",
            "Makefile",
            &["mk"],
            &["Makefile", "GNUmakefile"],
            &[],
            false,
        ),
        lc("markdown", "Markdown", &["md", "mdx"], &[], &[], false),
        lc("html", "HTML", &["html", "htm"], &[], &[], false),
        lc(
            "css",
            "CSS",
            &["css", "scss", "sass", "less"],
            &[],
            &[],
            false,
        ),
        lc("shell", "Shell", &["sh", "bash", "zsh"], &[], &[], false),
        lc("sql", "SQL", &["sql"], &[], &[], false),
        lc("graphql", "GraphQL", &["graphql", "gql"], &[], &[], false),
        lc("protobuf", "Protocol Buffers", &["proto"], &[], &[], false),
        lc("terraform", "Terraform", &["tf", "tfvars"], &[], &[], false),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_up_by_extension() {
        let r = LanguageRegistry::default_registry();
        let cfg = r.for_path(Path::new("src/main.rs")).unwrap();
        assert_eq!(cfg.id, "rust");
    }

    #[test]
    fn looks_up_by_filename_first() {
        let r = LanguageRegistry::default_registry();
        let cfg = r.for_path(Path::new("/abs/Dockerfile")).unwrap();
        assert_eq!(cfg.id, "dockerfile");
    }

    #[test]
    fn unknown_returns_none() {
        let r = LanguageRegistry::default_registry();
        assert!(r.for_path(Path::new("foo.xyz")).is_none());
    }

    #[test]
    fn jsx_and_tsx_routed() {
        let r = LanguageRegistry::default_registry();
        assert_eq!(r.for_path(Path::new("a.tsx")).unwrap().id, "typescript");
        assert_eq!(r.for_path(Path::new("a.jsx")).unwrap().id, "javascript");
    }
}
