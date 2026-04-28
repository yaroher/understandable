//! Tree-sitter-driven structural extraction + language and framework registry.
//!
//! Architecture:
//!   * [`AnalyzerPlugin`] is the per-file extraction trait;
//!   * [`PluginRegistry`] dispatches a path to the plugin that owns its
//!     language;
//!   * [`tree_sitter_plugin::TreeSitterPlugin`] is the default plugin and
//!     covers all tier-1 languages via `.scm` queries.

pub mod framework;
pub mod language;
pub mod parsers;
pub mod plugin;
pub mod registry;
pub mod structural;
pub mod tree_sitter_plugin;

pub use framework::{detect_frameworks, FrameworkConfig, FrameworkRegistry};
pub use language::{LanguageConfig, LanguageId, LanguageRegistry};
#[allow(deprecated)]
pub use plugin::{AnalyzerPlugin, PluginError};
pub use registry::PluginRegistry;
pub use structural::structural_hash;
pub use tree_sitter_plugin::TreeSitterPlugin;

/// Re-export the workspace-wide error so callers don't have to depend
/// on `ua_core` directly.
pub use ua_core::Error;

/// Convenience: build a registry pre-populated with the tree-sitter
/// plugin and the line-oriented non-code parsers.
pub fn default_registry() -> PluginRegistry {
    let mut reg = PluginRegistry::new();
    reg.register(Box::new(TreeSitterPlugin::new()));
    reg.register(Box::new(parsers::NonCodeParserPlugin::new()));
    reg
}
