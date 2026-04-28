//! Smoke test for `crate::builders::onboard_builder::build_onboarding_guide`.
//!
//! `ua-cli` is a binary-only crate, so the integration test brings the
//! source file in via `#[path]` rather than `use crate::…`. The included
//! module has no `crate::` references — it only depends on `ua_core` —
//! so this stays a one-line wiring change.

#[path = "../src/builders/onboard_builder.rs"]
mod onboard_builder;

use ua_core::{
    Complexity, GraphEdge, GraphKind, GraphNode, KnowledgeGraph, Layer, NodeType, ProjectMeta,
    TourStep,
};

fn file_node(id: &str, path: &str, name: &str, summary: &str, complexity: Complexity) -> GraphNode {
    GraphNode {
        id: id.into(),
        node_type: NodeType::File,
        name: name.into(),
        file_path: Some(path.into()),
        line_range: None,
        summary: summary.into(),
        tags: Vec::new(),
        complexity,
        language_notes: None,
        domain_meta: None,
        knowledge_meta: None,
    }
}

fn concept_node(id: &str, name: &str, summary: &str) -> GraphNode {
    GraphNode {
        id: id.into(),
        node_type: NodeType::Concept,
        name: name.into(),
        file_path: None,
        line_range: None,
        summary: summary.into(),
        tags: Vec::new(),
        complexity: Complexity::Moderate,
        language_notes: None,
        domain_meta: None,
        knowledge_meta: None,
    }
}

fn graph() -> KnowledgeGraph {
    let nodes = vec![
        file_node(
            "file:src/api/users.ts",
            "src/api/users.ts",
            "users.ts",
            "user CRUD endpoints",
            Complexity::Moderate,
        ),
        file_node(
            "file:src/services/billing.ts",
            "src/services/billing.ts",
            "billing.ts",
            "stripe + invoices",
            Complexity::Complex, // will surface in Hotspots
        ),
        concept_node(
            "concept:idempotency",
            "Idempotency",
            "Every webhook handler must be idempotent.",
        ),
    ];

    let layers = vec![
        Layer {
            id: "layer:api".into(),
            name: "API".into(),
            description: "HTTP entrypoints.".into(),
            node_ids: vec!["file:src/api/users.ts".into()],
        },
        Layer {
            id: "layer:service".into(),
            name: "Service".into(),
            description: "Business logic.".into(),
            node_ids: vec!["file:src/services/billing.ts".into()],
        },
    ];

    let tour = vec![TourStep {
        order: 1,
        title: "Start with the entrypoint".into(),
        description: "Read the API layer first.".into(),
        node_ids: vec!["file:src/api/users.ts".into()],
        language_lesson: Some("TS uses structural typing.".into()),
    }];

    KnowledgeGraph {
        version: "0.1.0".into(),
        kind: Some(GraphKind::Codebase),
        project: ProjectMeta {
            name: "demo".into(),
            languages: vec!["typescript".into()],
            frameworks: vec!["express".into()],
            description: "A demo project.".into(),
            analyzed_at: "2026-04-27T00:00:00Z".into(),
            git_commit_hash: "abc123".into(),
        },
        nodes,
        edges: Vec::<GraphEdge>::new(),
        layers,
        tour,
    }
}

#[test]
fn onboard_md_contains_expected_sections() {
    let g = graph();
    let md = onboard_builder::build_onboarding_guide(&g);

    // Top-level project header.
    assert!(
        md.starts_with("# demo"),
        "doc starts with project name: {md}"
    );

    // Required sections — the builder emits these as h2/h3 headers.
    for header in [
        "## Architecture",
        "### API",
        "### Service",
        "## Key Concepts",
        "### Idempotency",
        "## Getting Started",
        "### 1. Start with the entrypoint",
        "## File Map",
        "## Complexity Hotspots",
    ] {
        assert!(
            md.contains(header),
            "missing section header `{header}` in:\n{md}"
        );
    }

    // The hotspot bullet must call out the complex node by name.
    assert!(md.contains("billing.ts"), "complex node listed in hotspots");

    // The language lesson surfaces inside the tour step.
    assert!(
        md.contains("Language Tip"),
        "tour step surfaces the language lesson"
    );

    // File-map row formatted as expected.
    assert!(md.contains("`src/api/users.ts`"), "file map carries paths");
}

#[test]
fn onboard_md_skips_empty_sections_gracefully() {
    let mut g = graph();
    g.layers.clear();
    g.tour.clear();
    g.nodes.retain(|n| n.node_type == NodeType::File);

    let md = onboard_builder::build_onboarding_guide(&g);

    // Layers / Concepts / Tour are skipped entirely when their backing
    // collection is empty — this mirrors the original TS builder's
    // gating logic.
    assert!(!md.contains("## Architecture"));
    assert!(!md.contains("## Key Concepts"));
    assert!(!md.contains("## Getting Started"));
    // File map still renders.
    assert!(md.contains("## File Map"));
}
