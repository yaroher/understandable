//! Per-file extraction trait shared by every analyzer plugin.

use ua_core::{
    CallGraphEntry, Error, ImportResolution, ReferenceResolution, StructuralAnalysis,
};

/// Deprecated alias kept for one release so callers that imported
/// `ua_extract::PluginError` keep compiling. New code should depend
/// on [`ua_core::Error`] (also re-exported as
/// [`crate::Error`]).
#[deprecated(since = "0.2.0", note = "use `ua_core::Error` instead")]
pub type PluginError = Error;

/// Mirrors the TS `AnalyzerPlugin` interface.
pub trait AnalyzerPlugin: Send + Sync {
    fn name(&self) -> &'static str;
    /// Languages this plugin handles (matches `LanguageConfig::id`).
    fn languages(&self) -> &[&'static str];

    fn analyze_file(
        &self,
        language: &str,
        path: &str,
        content: &str,
    ) -> Result<StructuralAnalysis, Error>;

    fn extract_call_graph(
        &self,
        language: &str,
        path: &str,
        content: &str,
    ) -> Result<Vec<CallGraphEntry>, Error> {
        let _ = (language, path, content);
        Ok(Vec::new())
    }

    fn resolve_imports(
        &self,
        language: &str,
        path: &str,
        content: &str,
    ) -> Result<Vec<ImportResolution>, Error> {
        let _ = (language, path, content);
        Ok(Vec::new())
    }

    fn extract_references(
        &self,
        language: &str,
        path: &str,
        content: &str,
    ) -> Result<Vec<ReferenceResolution>, Error> {
        let _ = (language, path, content);
        Ok(Vec::new())
    }
}

// ---- internal constructors -------------------------------------------------

/// Build an `Error::Plugin` with the standard "no plugin for language X"
/// shape. Kept here so the message stays stable across plugins.
pub(crate) fn err_no_plugin(language: &str) -> Error {
    Error::Plugin(format!("no plugin for language {language}"))
}

/// `Error::Plugin` for a parser failure on `path`.
pub(crate) fn err_parse_failed(path: &str, message: impl Into<String>) -> Error {
    Error::Plugin(format!("parse failed for {path}: {}", message.into()))
}

/// `Error::Plugin` for tree-sitter query compilation / execution failure.
pub(crate) fn err_query(message: impl Into<String>) -> Error {
    Error::Plugin(format!("query: {}", message.into()))
}
