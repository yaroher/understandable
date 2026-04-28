//! Custom non-tree-sitter parsers for files where the ecosystem
//! grammars are stale or pre-0.20.
//!
//! Each parser is a small line-oriented function that consumes the raw
//! source and produces a [`StructuralAnalysis`]. The
//! [`NonCodeParserPlugin`] aggregates them so they slot into the same
//! [`PluginRegistry`] as the tree-sitter plugin — consumers don't have
//! to know which path produced the analysis.

use ua_core::{
    DefinitionInfo, EndpointInfo, Error, ResourceInfo, ServiceInfo, StepInfo, StructuralAnalysis,
};

use crate::plugin::AnalyzerPlugin;

pub mod dockerfile;
pub mod env;
pub mod graphql;
pub mod ini;
pub mod json;
pub mod makefile;
pub mod markdown;
pub mod protobuf;
pub mod shell;
pub mod sql;
pub mod terraform;
pub mod toml;
pub mod yaml;

pub struct NonCodeParserPlugin;

impl Default for NonCodeParserPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl NonCodeParserPlugin {
    pub fn new() -> Self {
        Self
    }
}

const HANDLED: &[&str] = &[
    "dockerfile",
    "makefile",
    "env",
    "ini",
    "yaml",
    "json",
    "toml",
    "markdown",
    "protobuf",
    "graphql",
    "shell",
    "sql",
    "terraform",
];

impl AnalyzerPlugin for NonCodeParserPlugin {
    fn name(&self) -> &'static str {
        "non-code-parsers"
    }

    fn languages(&self) -> &[&'static str] {
        HANDLED
    }

    fn analyze_file(
        &self,
        language: &str,
        path: &str,
        content: &str,
    ) -> Result<StructuralAnalysis, Error> {
        match language {
            "dockerfile" => Ok(dockerfile::analyze(content)),
            "makefile" => Ok(makefile::analyze(content)),
            "env" => Ok(env::analyze(content)),
            "ini" => Ok(ini::analyze(content)),
            "yaml" => Ok(yaml::analyze(content)),
            "json" => Ok(json::analyze(content)),
            "toml" => Ok(toml::analyze(content)),
            "markdown" => Ok(markdown::analyze(content)),
            "protobuf" => Ok(protobuf::analyze(content)),
            "graphql" => Ok(graphql::analyze(content)),
            "shell" => Ok(shell::analyze(content)),
            "sql" => Ok(sql::analyze(content)),
            "terraform" => Ok(terraform::analyze(content)),
            _ => Err(Error::Plugin(format!(
                "no plugin for language {language} (path={path})"
            ))),
        }
    }
}

pub(crate) fn def_with_fields(
    name: impl Into<String>,
    kind: &str,
    line: u32,
    fields: Vec<String>,
) -> DefinitionInfo {
    DefinitionInfo {
        name: name.into(),
        kind: kind.to_string(),
        line_range: (line, line),
        fields,
    }
}

pub(crate) fn endpoint(method: Option<&str>, path: &str, line: u32) -> EndpointInfo {
    EndpointInfo {
        method: method.map(|m| m.to_string()),
        path: path.to_string(),
        line_range: (line, line),
    }
}

pub(crate) fn step(name: impl Into<String>, line: u32) -> StepInfo {
    StepInfo {
        name: name.into(),
        line_range: (line, line),
    }
}

pub(crate) fn service(name: impl Into<String>, image: Option<&str>, line: u32) -> ServiceInfo {
    ServiceInfo {
        name: name.into(),
        image: image.map(|s| s.to_string()),
        ports: Vec::new(),
        line_range: Some((line, line)),
    }
}

pub(crate) fn resource(name: impl Into<String>, kind: &str, line: u32) -> ResourceInfo {
    ResourceInfo {
        name: name.into(),
        kind: kind.to_string(),
        line_range: (line, line),
    }
}
