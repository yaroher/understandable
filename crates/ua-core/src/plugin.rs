//! Mirror of the TS `AnalyzerPlugin` data shapes — the trait itself lives in
//! `ua-extract`, but the wire types stay here so they can be serialized into
//! intermediate JSON shared between the binary and Markdown agents.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SectionInfo {
    pub name: String,
    pub level: u32,
    pub line_range: (u32, u32),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DefinitionInfo {
    pub name: String,
    /// Parser-reported kind: "table" | "view" | "index" | "message" | "enum" | …
    pub kind: String,
    pub line_range: (u32, u32),
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ServiceInfo {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    pub ports: Vec<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_range: Option<(u32, u32)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EndpointInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    pub path: String,
    pub line_range: (u32, u32),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StepInfo {
    pub name: String,
    pub line_range: (u32, u32),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResourceInfo {
    pub name: String,
    pub kind: String,
    pub line_range: (u32, u32),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceResolution {
    pub source: String,
    pub target: String,
    /// "file" | "image" | "schema" | "service"
    pub reference_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FunctionDecl {
    pub name: String,
    pub line_range: (u32, u32),
    pub params: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_type: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ClassDecl {
    pub name: String,
    pub line_range: (u32, u32),
    pub methods: Vec<String>,
    pub properties: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImportDecl {
    pub source: String,
    pub specifiers: Vec<String>,
    pub line_number: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExportDecl {
    pub name: String,
    pub line_number: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StructuralAnalysis {
    pub functions: Vec<FunctionDecl>,
    pub classes: Vec<ClassDecl>,
    pub imports: Vec<ImportDecl>,
    pub exports: Vec<ExportDecl>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sections: Option<Vec<SectionInfo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definitions: Option<Vec<DefinitionInfo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub services: Option<Vec<ServiceInfo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoints: Option<Vec<EndpointInfo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steps: Option<Vec<StepInfo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<Vec<ResourceInfo>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImportResolution {
    pub source: String,
    pub resolved_path: String,
    pub specifiers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CallGraphEntry {
    pub caller: String,
    pub callee: String,
    pub line_number: u32,
}
