//! `Storage::file_modified_at` exposes the previously-written
//! `fingerprints.modified_at` column. The post-commit hook in the
//! binary uses this to decide which files to re-fingerprint without
//! re-hashing every byte.

use ua_persist::{blake3_string, Fingerprint, Storage};

#[tokio::test(flavor = "current_thread")]
async fn file_modified_at_returns_stored_value() {
    let s = Storage::open_fresh().await.unwrap();
    let prints = vec![
        Fingerprint {
            path: "src/lib.rs".into(),
            hash: blake3_string(b"hello"),
            modified_at: Some(1_700_000_000),
            structural_hash: None,
        },
        Fingerprint {
            path: "src/main.rs".into(),
            hash: blake3_string(b"world"),
            modified_at: None,
            structural_hash: None,
        },
    ];
    s.write_fingerprints(&prints).await.unwrap();

    assert_eq!(
        s.file_modified_at("src/lib.rs").await.unwrap(),
        Some(1_700_000_000)
    );
    assert_eq!(s.file_modified_at("src/main.rs").await.unwrap(), None);
    assert_eq!(s.file_modified_at("does/not/exist").await.unwrap(), None);
}
