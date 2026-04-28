//! Plugin registry — routes a language id to the registered plugin.

use std::collections::HashMap;

use ua_core::{CallGraphEntry, Error, StructuralAnalysis};

use crate::plugin::{err_no_plugin, AnalyzerPlugin};
use crate::structural::structural_hash;

#[derive(Default)]
pub struct PluginRegistry {
    plugins: Vec<Box<dyn AnalyzerPlugin>>,
    by_lang: HashMap<String, usize>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, plugin: Box<dyn AnalyzerPlugin>) {
        let idx = self.plugins.len();
        for lang in plugin.languages() {
            self.by_lang.insert((*lang).to_string(), idx);
        }
        self.plugins.push(plugin);
    }

    pub fn supports(&self, language: &str) -> bool {
        self.by_lang.contains_key(language)
    }

    pub fn analyze_file(
        &self,
        language: &str,
        path: &str,
        content: &str,
    ) -> Result<StructuralAnalysis, Error> {
        let idx = self
            .by_lang
            .get(language)
            .ok_or_else(|| err_no_plugin(language))?;
        self.plugins[*idx].analyze_file(language, path, content)
    }

    pub fn extract_call_graph(
        &self,
        language: &str,
        path: &str,
        content: &str,
    ) -> Result<Vec<CallGraphEntry>, Error> {
        let Some(idx) = self.by_lang.get(language) else {
            return Ok(Vec::new());
        };
        self.plugins[*idx].extract_call_graph(language, path, content)
    }

    /// Run the analyser and return only the deterministic structural
    /// hash. Returns `None` for unsupported languages or when the
    /// analysis itself fails (e.g. tree-sitter parser error on
    /// pathological input). Callers should fall back to a plain
    /// blake3 of the file bytes when they need a non-empty hash.
    ///
    /// `path` is forwarded to the underlying plugin only for error
    /// reporting; the hash itself depends on `content` plus the
    /// language id.
    pub fn structural_hash_of(&self, language: &str, path: &str, content: &str) -> Option<String> {
        let analysis = self.analyze_file(language, path, content).ok()?;
        Some(structural_hash(&analysis))
    }
}
