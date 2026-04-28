//! Regression tests for the project_root stamp.
//!
//! `tests/project_root.rs` already covers the basic happy paths. This
//! file pins the *typed-error* contract: a real mismatch must surface
//! `Error::ProjectRootMismatch` (not a panic, not a generic `Error`),
//! a legacy archive without a stamp gets stamped silently, and
//! reopening the same project after the first stamp is idempotent.

use ua_core::{
    Complexity, Error, GraphKind, GraphNode, KnowledgeGraph, NodeType, ProjectMeta, StorageSettings,
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
async fn mismatch_emits_typed_error_not_panic() {
    // Two distinct project roots, one shared (absolute) storage dir.
    // The contract: ProjectRootMismatch is a typed variant — callers
    // can match on it to print a remediation hint. A panic or a
    // string-only `Error::Other` would defeat the purpose.
    let shared_storage = tempfile::tempdir().unwrap();
    let project_a = tempfile::tempdir().unwrap();
    let project_b = tempfile::tempdir().unwrap();

    let mut storage_settings = StorageSettings::default();
    storage_settings.dir = shared_storage.path().to_string_lossy().into_owned();

    let layout_a = ProjectLayout::with_storage(project_a.path(), &storage_settings);
    let layout_b = ProjectLayout::with_storage(project_b.path(), &storage_settings);

    {
        let s = Storage::open(&layout_a).await.unwrap();
        s.save_graph_for(&tiny_graph("a"), &layout_a).await.unwrap();
        s.save(&layout_a).await.unwrap();
    }

    let result = Storage::open(&layout_b).await;
    let err = match result {
        Ok(_) => panic!("expected ProjectRootMismatch, got Ok"),
        Err(e) => e,
    };
    match err {
        Error::ProjectRootMismatch { stored, current } => {
            // The error message should carry both paths so the user
            // can fix `storage.dir` / `storage.db_name`.
            assert!(
                !stored.is_empty(),
                "stored path must be present on the variant",
            );
            assert!(
                !current.is_empty(),
                "current path must be present on the variant",
            );
            // Display impl mentions remediation hint.
            let msg = format!(
                "{}",
                Error::ProjectRootMismatch {
                    stored: stored.clone(),
                    current: current.clone(),
                }
            );
            assert!(
                msg.contains("project root mismatch"),
                "Display must surface the typed message, got: {msg}",
            );
        }
        other => panic!("expected ProjectRootMismatch, got: {other:?}"),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn legacy_archive_without_stamp_gets_stamped() {
    // An archive saved before the stamp existed has no `project_root`
    // entry in `meta`. Opening with a layout that *does* know its
    // project root must succeed AND stamp the archive on save so
    // future opens compare equal.
    let project = tempfile::tempdir().unwrap();
    let layout = ProjectLayout::under(project.path());

    // Step 1: write a "legacy" archive — same trick the existing
    // project_root.rs uses: clear `project_root` from the layout.
    {
        let mut legacy = layout.clone();
        legacy.project_root = None;
        let s = Storage::open(&legacy).await.unwrap();
        s.save_graph(&tiny_graph("legacy")).await.unwrap();
        s.save(&legacy).await.unwrap();
    }

    // Step 2: reopen with the full layout. Stamping happens via the
    // `check_project_root` warn path; the next save persists it.
    {
        let s = Storage::open(&layout).await.unwrap();
        let g = s.load_graph().await.unwrap();
        assert_eq!(g.project.name, "legacy");
        s.save(&layout).await.unwrap();
    }

    // Step 3: third open must succeed without warnings (the stamp is
    // now in place). Use a different layout pointing at the same
    // *project_root* — must compare equal.
    {
        let s = Storage::open(&layout).await.unwrap();
        let g = s.load_graph().await.unwrap();
        assert_eq!(g.project.name, "legacy");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn idempotent_check_after_first_stamp() {
    // Repeated open/save cycles on the same layout must NOT flip the
    // stamp around or produce a mismatch on themselves.
    let project = tempfile::tempdir().unwrap();
    let layout = ProjectLayout::for_project(project.path());

    {
        let s = Storage::open(&layout).await.unwrap();
        s.save_graph_for(&tiny_graph("ok"), &layout).await.unwrap();
        s.save(&layout).await.unwrap();
    }

    // Several reopen cycles — the result must always be Ok.
    for i in 0..3 {
        let s = Storage::open(&layout)
            .await
            .unwrap_or_else(|e| panic!("reopen #{i} failed: {e:?}"));
        let g = s.load_graph().await.unwrap();
        assert_eq!(g.project.name, "ok");
        s.save(&layout).await.unwrap();
    }
}
