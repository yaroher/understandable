//! Verify that `Storage::save_kind` writes the .zst atomically: a
//! crash mid-write should leave the previous good `.db.zst` intact.
//! We can't actually SIGKILL the process from a test, so we simulate
//! the failure mode by:
//!
//!   1. Producing a real, valid `.db.zst` for project A.
//!   2. Writing a corrupt `.db.zst.tmp` file (simulating a crashed
//!      future save).
//!   3. Re-opening — the stale tmp must be ignored and the previous
//!      `.db.zst` must still load cleanly.

use ua_core::{
    Complexity, EdgeDirection, EdgeType, GraphEdge, GraphKind, GraphNode, KnowledgeGraph,
    Layer, NodeType, ProjectMeta, TourStep,
};
use ua_persist::{ProjectLayout, Storage};

fn graph(name: &str) -> KnowledgeGraph {
    KnowledgeGraph {
        version: "0.1.0".into(),
        kind: Some(GraphKind::Codebase),
        project: ProjectMeta {
            name: name.into(),
            languages: vec!["rust".into()],
            frameworks: vec![],
            description: "atomic test".into(),
            analyzed_at: "2026-04-28T00:00:00Z".into(),
            git_commit_hash: "deadbeef".into(),
        },
        nodes: vec![GraphNode {
            id: format!("file:{name}.rs"),
            node_type: NodeType::File,
            name: format!("{name}.rs"),
            file_path: Some(format!("{name}.rs")),
            line_range: None,
            summary: "the only node".into(),
            tags: vec!["unique".into()],
            complexity: Complexity::Simple,
            language_notes: None,
            domain_meta: None,
            knowledge_meta: None,
        }],
        edges: vec![GraphEdge {
            source: format!("file:{name}.rs"),
            target: format!("file:{name}.rs"),
            edge_type: EdgeType::Related,
            direction: EdgeDirection::Forward,
            description: None,
            weight: 1.0,
        }],
        layers: vec![Layer {
            id: "layer:l".into(),
            name: "L".into(),
            description: "".into(),
            node_ids: vec![format!("file:{name}.rs")],
        }],
        tour: vec![TourStep {
            order: 1,
            title: "t".into(),
            description: "".into(),
            node_ids: vec![format!("file:{name}.rs")],
            language_lesson: None,
        }],
    }
}

#[tokio::test(flavor = "current_thread")]
async fn stale_tmp_does_not_break_open() {
    let project = tempfile::tempdir().unwrap();
    let layout = ProjectLayout::under(project.path());

    // 1. First, produce a real .db.zst for the project.
    {
        let s = Storage::open(&layout).await.unwrap();
        s.save_graph(&graph("alpha")).await.unwrap();
        s.save(&layout).await.unwrap();
    }
    let zst_path = layout.graph_archive();
    assert!(zst_path.exists());
    let real_bytes = std::fs::read(&zst_path).unwrap();
    assert!(!real_bytes.is_empty());

    // 2. Plant a corrupt .tmp next to it (simulates a SIGKILL-mid-save).
    let tmp_path = {
        let mut s = zst_path.as_os_str().to_os_string();
        s.push(".tmp");
        std::path::PathBuf::from(s)
    };
    std::fs::write(&tmp_path, b"NOT A VALID ZSTD STREAM").unwrap();
    assert!(tmp_path.exists());

    // 3. Open should still succeed and the loaded graph should be the
    //    pre-crash one.
    let s = Storage::open(&layout).await.unwrap();
    let g = s.load_graph().await.unwrap();
    assert_eq!(g.project.name, "alpha");
    assert_eq!(g.nodes.len(), 1);
    assert_eq!(g.nodes[0].id, "file:alpha.rs");

    // The stale tmp should still be sitting there — open shouldn't have
    // touched it.
    assert!(tmp_path.exists());
}

#[tokio::test(flavor = "current_thread")]
async fn save_overwrites_with_atomic_rename() {
    // Verify there is no temp file left over after a clean save.
    let project = tempfile::tempdir().unwrap();
    let layout = ProjectLayout::under(project.path());
    let s = Storage::open(&layout).await.unwrap();
    s.save_graph(&graph("beta")).await.unwrap();
    s.save(&layout).await.unwrap();

    let zst_path = layout.graph_archive();
    let tmp_path = {
        let mut s = zst_path.as_os_str().to_os_string();
        s.push(".tmp");
        std::path::PathBuf::from(s)
    };
    assert!(zst_path.exists());
    assert!(!tmp_path.exists(), "atomic_write must remove the .tmp file via rename");
}
