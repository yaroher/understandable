//! Framework detection — port of `framework-registry.ts`.
//!
//! Detection works in two passes:
//!   1. *Manifest* — match a known manifest file (e.g. `package.json`,
//!      `requirements.txt`) and look for any of the framework's keywords
//!      in its raw bytes.
//!   2. *Source* — match an `import` line containing one of the keywords
//!      (handled by the analyzer when it sees imports).
//!
//! Keywords are intentionally short and case-sensitive — they target
//! package names, not human strings.

use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FrameworkConfig {
    pub id: String,
    pub display_name: String,
    pub languages: Vec<String>,
    pub detection_keywords: Vec<String>,
    pub manifest_files: Vec<String>,
}

#[derive(Debug, Default, Clone)]
pub struct FrameworkRegistry {
    items: Vec<FrameworkConfig>,
}

impl FrameworkRegistry {
    pub fn default_registry() -> Self {
        Self { items: builtins() }
    }

    pub fn all(&self) -> &[FrameworkConfig] {
        &self.items
    }

    /// Frameworks whose manifest file matches `path` *and* whose keywords
    /// appear in `content`.
    pub fn detect_in_manifest(&self, path: &Path, content: &str) -> Vec<&FrameworkConfig> {
        let basename = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        self.items
            .iter()
            .filter(|fw| {
                fw.manifest_files
                    .iter()
                    .any(|m| m.eq_ignore_ascii_case(basename))
                    && fw.detection_keywords.iter().any(|kw| content.contains(kw))
            })
            .collect()
    }

    /// Frameworks whose detection keywords appear in any of the supplied
    /// import source strings.
    pub fn detect_in_imports<'a, I, S>(&self, imports: I) -> Vec<&FrameworkConfig>
    where
        I: IntoIterator<Item = &'a S>,
        S: AsRef<str> + 'a + ?Sized,
    {
        let sources: Vec<&str> = imports.into_iter().map(|s| s.as_ref()).collect();
        self.items
            .iter()
            .filter(|fw| {
                fw.detection_keywords
                    .iter()
                    .any(|kw| sources.iter().any(|s| s.contains(kw.as_str())))
            })
            .collect()
    }
}

/// Aggregate detection helper used by the project scanner.
pub fn detect_frameworks(
    registry: &FrameworkRegistry,
    manifests: &[(&Path, &str)],
    imports: &[&str],
) -> Vec<String> {
    let mut found: BTreeSet<String> = BTreeSet::new();
    for (path, content) in manifests {
        for fw in registry.detect_in_manifest(path, content) {
            found.insert(fw.display_name.clone());
        }
    }
    for fw in registry.detect_in_imports(imports.iter().copied()) {
        found.insert(fw.display_name.clone());
    }
    found.into_iter().collect()
}

fn fw(
    id: &str,
    display: &str,
    langs: &[&str],
    keywords: &[&str],
    manifests: &[&str],
) -> FrameworkConfig {
    FrameworkConfig {
        id: id.to_string(),
        display_name: display.to_string(),
        languages: langs.iter().map(|s| s.to_string()).collect(),
        detection_keywords: keywords.iter().map(|s| s.to_string()).collect(),
        manifest_files: manifests.iter().map(|s| s.to_string()).collect(),
    }
}

fn builtins() -> Vec<FrameworkConfig> {
    vec![
        fw(
            "react",
            "React",
            &["javascript", "typescript"],
            &["\"react\"", "from 'react'", "from \"react\""],
            &["package.json"],
        ),
        fw(
            "vue",
            "Vue",
            &["javascript", "typescript"],
            &["\"vue\"", "from 'vue'", "from \"vue\""],
            &["package.json"],
        ),
        fw(
            "next",
            "Next.js",
            &["javascript", "typescript"],
            &["\"next\"", "from 'next/", "from \"next/"],
            &["package.json"],
        ),
        fw(
            "express",
            "Express",
            &["javascript", "typescript"],
            &["\"express\"", "require('express')", "from 'express'"],
            &["package.json"],
        ),
        fw(
            "flask",
            "Flask",
            &["python"],
            &["Flask", "flask", "from flask"],
            &["requirements.txt", "pyproject.toml", "Pipfile"],
        ),
        fw(
            "django",
            "Django",
            &["python"],
            &["Django", "django", "from django"],
            &["requirements.txt", "pyproject.toml", "Pipfile"],
        ),
        fw(
            "fastapi",
            "FastAPI",
            &["python"],
            &["fastapi", "from fastapi"],
            &["requirements.txt", "pyproject.toml", "Pipfile"],
        ),
        fw(
            "rails",
            "Rails",
            &["ruby"],
            &["rails", "Rails"],
            &["Gemfile", "config/application.rb"],
        ),
        fw(
            "spring",
            "Spring",
            &["java"],
            &["org.springframework", "spring-boot"],
            &["pom.xml", "build.gradle", "build.gradle.kts"],
        ),
        fw(
            "gin",
            "Gin",
            &["go"],
            &["github.com/gin-gonic/gin"],
            &["go.mod"],
        ),
        fw(
            "actix",
            "Actix",
            &["rust"],
            &["actix-web", "actix_web"],
            &["Cargo.toml"],
        ),
        fw("axum", "Axum", &["rust"], &["axum"], &["Cargo.toml"]),
        fw("rocket", "Rocket", &["rust"], &["rocket"], &["Cargo.toml"]),
    ]
}
