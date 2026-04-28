//! Live-reload integration test.
//!
//! Verifies that the SSE `/api/events` endpoint delivers a `graph-reloaded`
//! event to a connected client within a bounded timeout after the broadcast
//! channel fires.
//!
//! We don't write a real archive to disk here — instead we send on the
//! broadcast channel directly so the test is self-contained. A separate
//! `#[ignore]`d test documents the full file-watcher path.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ua_core::{KnowledgeGraph, ProjectMeta};
use ua_server::state::ReloadEvent;
use ua_server::{router_for, AppState};

fn empty_graph(name: &str) -> KnowledgeGraph {
    KnowledgeGraph::new(ProjectMeta {
        name: name.into(),
        languages: vec![],
        frameworks: vec![],
        description: String::new(),
        analyzed_at: String::new(),
        git_commit_hash: String::new(),
    })
}

/// Boot the server and return (base_url, Arc<AppState>, join_handle).
async fn boot(state: AppState) -> (String, Arc<AppState>, tokio::task::JoinHandle<()>) {
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    let bound = listener.local_addr().unwrap();
    let state = Arc::new(state);
    let app = router_for(Arc::clone(&state), bound);
    let base = format!("http://{bound}");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (base, state, handle)
}

/// Subscribe to `/api/events` via a raw TCP connection and return the first
/// `data:` line received within `timeout`.
async fn first_data_line(host: &str, port: u16, timeout: Duration) -> Option<String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let mut stream = tokio::time::timeout(timeout, tokio::net::TcpStream::connect((host, port)))
        .await
        .ok()?
        .ok()?;

    let request = format!(
        "GET /api/events HTTP/1.1\r\nHost: {host}:{port}\r\nAccept: text/event-stream\r\nConnection: keep-alive\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await.ok()?;
    stream.flush().await.ok()?;

    let mut reader = BufReader::new(stream);
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        if tokio::time::Instant::now() >= deadline {
            return None;
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        let mut line = String::new();
        let n = match tokio::time::timeout(remaining, reader.read_line(&mut line)).await {
            Ok(Ok(n)) => n,
            _ => return None,
        };
        if n == 0 {
            return None; // EOF
        }
        let trimmed = line.trim_end_matches(['\r', '\n']).to_string();
        if trimmed.starts_with("data:") {
            return Some(trimmed);
        }
    }
}

#[tokio::test]
async fn events_endpoint_delivers_reload_notification() {
    let state = AppState::with_graphs(empty_graph("live-test"), None, None, PathBuf::new());
    let (base, state_arc, handle) = boot(state).await;

    // Parse host/port from the base URL (format: "http://127.0.0.1:<port>").
    let without_scheme = base.trim_start_matches("http://");
    let (host, port_str) = without_scheme.rsplit_once(':').expect("base has port");
    let port: u16 = port_str.parse().unwrap();

    // Give the server a moment to start accepting connections.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Spawn a task that opens the SSE stream and waits for the first data line.
    let host = host.to_string();
    let event_task =
        tokio::spawn(async move { first_data_line(&host, port, Duration::from_secs(3)).await });

    // Let the subscriber connect before we fire.
    tokio::time::sleep(Duration::from_millis(150)).await;

    // Trigger a reload event directly via the broadcast channel.
    let _ = state_arc.tx.send(ReloadEvent {
        kind: "codebase".to_string(),
    });

    let result = event_task.await.expect("SSE task panicked");
    assert!(
        result.is_some(),
        "expected a data: line from /api/events within 3 s"
    );
    let data_line = result.unwrap();
    assert!(
        data_line.contains("codebase"),
        "expected 'codebase' in SSE data, got: {data_line}"
    );

    handle.abort();
}

/// Full file-watcher path: write an archive to disk and wait for the SSE event.
///
/// Marked `#[ignore]` because it requires `ua-persist` write + zstd stack
/// and can be slow; run explicitly with `cargo test -- --ignored`.
#[tokio::test]
#[ignore = "requires ua-persist write stack; run with --ignored to exercise file-watcher path"]
async fn file_watcher_triggers_reload() {
    // This test would:
    // 1. Create a tempdir storage layout.
    // 2. Boot the server pointing at that layout.
    // 3. Connect to /api/events.
    // 4. Write a new .tar.zst archive (via Storage::save_kind).
    // 5. Assert the SSE event arrives within 2 s.
    // 6. Assert /api/graph returns the updated graph.
    //
    // The broadcast path is already covered by
    // `events_endpoint_delivers_reload_notification`.
    todo!("implement when ua-persist test helpers are stabilised");
}
