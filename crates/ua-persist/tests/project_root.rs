//! Project-root guard: two projects sharing one absolute storage dir
//! must not silently overwrite each other.

use ua_core::{
    Complexity, Error, GraphKind, GraphNode, KnowledgeGraph, NodeType, ProjectMeta,
    StorageSettings,
};
use ua_persist::{ProjectLayout, Storage};

fn tiny_graph(name: &str) -> KnowledgeGraph {
    KnowledgeGraph {
        version: "0.1.0".into(),
        kind: Some(GraphKind::Codebase),
        project: ProjectMeta {
            name: name.into(),
            languages: vec!["rust".into()],
            frameworks: vec![],
            description: "".into(),
            analyzed_at: "2026-04-28T00:00:00Z".into(),
            git_commit_hash: "abc".into(),
        },
        nodes: vec![GraphNode {
            id: "file:lib.rs".into(),
            node_type: NodeType::File,
            name: "lib.rs".into(),
            file_path: Some("lib.rs".into()),
            line_range: None,
            summary: "".into(),
            tags: vec![],
            complexity: Complexity::Simple,
            language_notes: None,
            domain_meta: None,
            knowledge_meta: None,
        }],
        edges: vec![],
        layers: vec![],
        tour: vec![],
    }
}

#[tokio::test(flavor = "current_thread")]
async fn second_project_pointing_at_same_storage_dir_is_rejected() {
    // Two distinct project roots, one shared (absolute) storage dir.
    let shared_storage = tempfile::tempdir().unwrap();
    let project_a = tempfile::tempdir().unwrap();
    let project_b = tempfile::tempdir().unwrap();

    let mut storage_settings = StorageSettings::default();
    storage_settings.dir = shared_storage.path().to_string_lossy().into_owned();

    let layout_a = ProjectLayout::with_storage(project_a.path(), &storage_settings);
    let layout_b = ProjectLayout::with_storage(project_b.path(), &storage_settings);

    // Project A saves first — stamps its own root.
    {
        let s = Storage::open(&layout_a).await.unwrap();
        s.save_graph(&tiny_graph("a")).await.unwrap();
        s.stamp_project_root(&layout_a).await.unwrap();
        s.save(&layout_a).await.unwrap();
    }

    // Project B tries to open the same storage dir. Open MUST fail
    // with ProjectRootMismatch.
    let result = Storage::open(&layout_b).await;
    match result {
        Err(Error::ProjectRootMismatch { stored, current }) => {
            assert!(stored.contains(project_a.path().to_string_lossy().as_ref()));
            assert!(current.contains(project_b.path().to_string_lossy().as_ref()));
        }
        Err(other) => panic!("expected ProjectRootMismatch, got: {other:?}"),
        Ok(_) => panic!("expected ProjectRootMismatch, but open succeeded"),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn legacy_db_without_stamp_gets_upgraded_silently() {
    // A DB saved by an older version has no `project_root` row in
    // `meta`. Opening with a fresh layout should write the row in and
    // then succeed.
    let project = tempfile::tempdir().unwrap();
    let layout = ProjectLayout::under(project.path());

    // 1. Make a "legacy" DB by saving with a layout that has no
    //    project_root recorded — i.e. construct via the Default ctor
    //    used by tests.
    {
        let mut legacy = layout.clone();
        legacy.project_root = None;
        let s = Storage::open(&legacy).await.unwrap();
        s.save_graph(&tiny_graph("legacy")).await.unwrap();
        s.save(&legacy).await.unwrap();
    }

    // 2. Reopen with a layout that DOES carry a project_root. Should
    //    succeed; the warn-on-stamp branch fires once.
    let s = Storage::open(&layout).await.unwrap();
    let g = s.load_graph().await.unwrap();
    assert_eq!(g.project.name, "legacy");
}

#[tokio::test(flavor = "current_thread")]
async fn same_project_reopen_is_idempotent() {
    let project = tempfile::tempdir().unwrap();
    let layout = ProjectLayout::for_project(project.path());

    {
        let s = Storage::open(&layout).await.unwrap();
        s.save_graph(&tiny_graph("ok")).await.unwrap();
        s.stamp_project_root(&layout).await.unwrap();
        s.save(&layout).await.unwrap();
    }

    // Re-open with the same layout — must NOT fail.
    let s = Storage::open(&layout).await.unwrap();
    let g = s.load_graph().await.unwrap();
    assert_eq!(g.project.name, "ok");
}
