use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMeta {
    pub name: String,
    pub languages: Vec<String>,
    pub frameworks: Vec<String>,
    pub description: String,
    pub analyzed_at: String,
    pub git_commit_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThemeConfig {
    pub preset_id: String,
    pub accent_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisMeta {
    pub last_analyzed_at: String,
    pub git_commit_hash: String,
    pub version: String,
    pub analyzed_files: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<ThemeConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectConfig {
    pub auto_update: bool,
}
