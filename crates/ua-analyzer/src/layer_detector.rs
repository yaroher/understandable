//! Heuristic + LLM-driven layer detection — port of `analyzer/layer-detector.ts`.
//!
//! ## Behaviour notes
//!
//! - `apply_llm_layers` is defensive against duplicate `name` entries in
//!   the LLM response. Two layers with the same name no longer panic on
//!   `unwrap()`; instead they merge into a single `BTreeMap` bucket using
//!   `entry(...).or_default()`, and node assignment uses
//!   `if let Some(v) = layer_map.get_mut(...)` so a missing bucket is a
//!   no-op rather than a crash.

use ua_core::{KnowledgeGraph, Layer, NodeType};

#[derive(Debug, Clone)]
struct LayerPattern {
    patterns: &'static [&'static str],
    layer_name: &'static str,
    description: &'static str,
}

const LAYER_PATTERNS: &[LayerPattern] = &[
    LayerPattern {
        patterns: &["routes", "controller", "handler", "endpoint", "api"],
        layer_name: "API Layer",
        description: "HTTP endpoints, route handlers, and API controllers",
    },
    LayerPattern {
        patterns: &["service", "usecase", "use-case", "business"],
        layer_name: "Service Layer",
        description: "Business logic and application services",
    },
    LayerPattern {
        patterns: &[
            "model",
            "entity",
            "schema",
            "database",
            "db",
            "migration",
            "repository",
            "repo",
        ],
        layer_name: "Data Layer",
        description: "Data models, database access, and persistence",
    },
    LayerPattern {
        patterns: &[
            "component",
            "view",
            "page",
            "screen",
            "layout",
            "widget",
            "ui",
        ],
        layer_name: "UI Layer",
        description: "User interface components and views",
    },
    LayerPattern {
        patterns: &["middleware", "interceptor", "guard", "filter", "pipe"],
        layer_name: "Middleware Layer",
        description: "Request/response middleware and interceptors",
    },
    LayerPattern {
        patterns: &[
            "client",
            "integration",
            "external",
            "sdk",
            "vendor",
            "adapter",
        ],
        layer_name: "External Services",
        description: "External service integrations, SDKs, and third-party adapters",
    },
    LayerPattern {
        patterns: &[
            "worker",
            "job",
            "queue",
            "cron",
            "consumer",
            "processor",
            "scheduler",
            "background",
        ],
        layer_name: "Background Tasks",
        description: "Background workers, job processors, and scheduled tasks",
    },
    LayerPattern {
        patterns: &["util", "helper", "lib", "common", "shared"],
        layer_name: "Utility Layer",
        description: "Shared utilities, helpers, and common libraries",
    },
    LayerPattern {
        patterns: &[
            "test",
            "spec",
            "__test__",
            "__spec__",
            "__tests__",
            "__specs__",
        ],
        layer_name: "Test Layer",
        description: "Test files and test utilities",
    },
    LayerPattern {
        patterns: &["config", "setting", "env"],
        layer_name: "Configuration Layer",
        description: "Application configuration and environment settings",
    },
];

fn to_layer_id(name: &str) -> String {
    let kebab: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_whitespace() { '-' } else { c })
        .collect();
    format!("layer:{kebab}")
}

fn match_file_to_layer(file_path: &str) -> Option<&'static str> {
    let normalized = file_path.replace('\\', "/").to_lowercase();
    let segments: Vec<&str> = normalized.split('/').collect();
    for pattern in LAYER_PATTERNS {
        for segment in &segments {
            for p in pattern.patterns {
                if *segment == *p || *segment == format!("{p}s") {
                    return Some(pattern.layer_name);
                }
            }
        }
    }
    None
}

/// Heuristic layer detection — assigns file nodes to layers based on
/// directory path patterns. Files that don't match any pattern fall into
/// a synthesized "Core" layer.
pub fn detect_layers(graph: &KnowledgeGraph) -> Vec<Layer> {
    use std::collections::BTreeMap;
    let mut layer_map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for node in &graph.nodes {
        if node.node_type != NodeType::File {
            continue;
        }
        let layer_name = node
            .file_path
            .as_deref()
            .and_then(match_file_to_layer)
            .unwrap_or("Core");
        layer_map
            .entry(layer_name.to_string())
            .or_default()
            .push(node.id.clone());
    }
    layer_map
        .into_iter()
        .map(|(name, node_ids)| {
            let description = if name == "Core" {
                "Core application files".to_string()
            } else {
                LAYER_PATTERNS
                    .iter()
                    .find(|p| p.layer_name == name)
                    .map(|p| p.description.to_string())
                    .unwrap_or_default()
            };
            Layer {
                id: to_layer_id(&name),
                name,
                description,
                node_ids,
            }
        })
        .collect()
}

/// Layer description shape returned by the LLM.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct LlmLayerResponse {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, rename = "filePatterns")]
    pub file_patterns: Vec<String>,
}

/// Build the prompt sent to the LLM for layer detection.
pub fn build_layer_detection_prompt(graph: &KnowledgeGraph) -> String {
    let mut paths = Vec::new();
    for node in &graph.nodes {
        if node.node_type == NodeType::File {
            if let Some(p) = &node.file_path {
                paths.push(p.as_str());
            }
        }
    }
    let file_list = paths
        .iter()
        .map(|p| format!("  - {p}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "You are a software architecture analyst. Given the following list of file paths from a codebase, identify the logical architectural layers.\n\nFile paths:\n{file_list}\n\nReturn a JSON array of 3-7 layers. Each layer object must have:\n- \"name\": A short layer name (e.g., \"API\", \"Data\", \"UI\")\n- \"description\": What this layer is responsible for (1 sentence)\n- \"filePatterns\": An array of path prefixes that belong to this layer (e.g., [\"src/routes/\", \"src/controllers/\"])\n\nEvery file should belong to exactly one layer. Use the most specific pattern possible.\n\nRespond ONLY with the JSON array, no additional text.",
    )
}

/// Tolerant parser for the LLM's layer-detection response. Strips markdown
/// fences and salvages the first JSON array. Returns `None` on hard
/// failure.
pub fn parse_layer_detection_response(response: &str) -> Option<Vec<LlmLayerResponse>> {
    let trimmed = response.trim();
    if trimmed.is_empty() {
        return None;
    }
    let json_str = strip_code_fence(trimmed);
    let array_str = extract_first_array(json_str)?;
    let parsed: Vec<serde_json::Value> = serde_json::from_str(array_str).ok()?;
    let mut out = Vec::new();
    for item in parsed {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let Some(name) = obj.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        let description = obj
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let file_patterns = obj
            .get("filePatterns")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        out.push(LlmLayerResponse {
            name: name.to_string(),
            description,
            file_patterns,
        });
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn strip_code_fence(s: &str) -> &str {
    let s = s.trim();
    if let Some(after) = s.strip_prefix("```json") {
        return after
            .trim_start_matches('\n')
            .trim()
            .trim_end_matches("```")
            .trim();
    }
    if let Some(after) = s.strip_prefix("```") {
        return after
            .trim_start_matches('\n')
            .trim()
            .trim_end_matches("```")
            .trim();
    }
    s
}

fn extract_first_array(s: &str) -> Option<&str> {
    let start = s.find('[')?;
    let end = s.rfind(']')?;
    if end <= start {
        return None;
    }
    Some(&s[start..=end])
}

/// Apply LLM-supplied layer definitions to the graph. Files match a layer
/// when their path starts with a `filePatterns` entry or contains it
/// after a `/` separator. Unassigned files land in an "Other" layer.
///
/// Robust against duplicate `LlmLayerResponse.name` entries: layers
/// sharing a name merge into one bucket, and node assignment never
/// panics on a missing entry.
pub fn apply_llm_layers(graph: &KnowledgeGraph, llm_layers: &[LlmLayerResponse]) -> Vec<Layer> {
    use std::collections::BTreeMap;
    let mut layer_map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    // Pre-create one bucket per unique name. `entry().or_default()` is
    // idempotent, so two LLM layers with the same name share a bucket.
    for l in llm_layers {
        layer_map.entry(l.name.clone()).or_default();
    }
    for node in &graph.nodes {
        if node.node_type != NodeType::File {
            continue;
        }
        let Some(path) = &node.file_path else {
            layer_map
                .entry("Other".into())
                .or_default()
                .push(node.id.clone());
            continue;
        };
        let normalized = path.replace('\\', "/");
        let mut assigned = false;
        for l in llm_layers {
            for pattern in &l.file_patterns {
                if normalized.starts_with(pattern) || normalized.contains(&format!("/{pattern}")) {
                    // `entry().or_default()` instead of
                    // `get_mut().unwrap()` — defensive even though we
                    // pre-seeded the map: avoids a panic if the layer
                    // bucket goes missing for any reason.
                    if let Some(v) = layer_map.get_mut(&l.name) {
                        v.push(node.id.clone());
                    } else {
                        layer_map
                            .entry(l.name.clone())
                            .or_default()
                            .push(node.id.clone());
                    }
                    assigned = true;
                    break;
                }
            }
            if assigned {
                break;
            }
        }
        if !assigned {
            layer_map
                .entry("Other".into())
                .or_default()
                .push(node.id.clone());
        }
    }
    let mut out = Vec::new();
    for (name, node_ids) in layer_map {
        if node_ids.is_empty() {
            continue;
        }
        // Use the first matching `LlmLayerResponse` for the description.
        // When two LLM layers share a name, the first wins — predictable
        // and matches the previous behaviour for unique-name inputs.
        let description = llm_layers
            .iter()
            .find(|l| l.name == name)
            .map(|l| l.description.clone())
            .unwrap_or_else(|| "Uncategorized files".to_string());
        out.push(Layer {
            id: to_layer_id(&name),
            name,
            description,
            node_ids,
        });
    }
    out
}
