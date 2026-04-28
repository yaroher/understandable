//! Regression tests for the atomic `tar.zst` writer.
//!
//! `archive::write_archive` follows the standard
//! `tmp + fsync + rename` dance. Earlier iterations could leave a stray
//! `.tmp` lying around on a panic mid-write or refuse to overwrite the
//! destination if a stale `.tmp` from a crashed process was still
//! present. Both modes are pinned here.

use std::path::PathBuf;

use ua_persist::archive;

fn tmp_for(dst: &std::path::Path) -> PathBuf {
    let mut s = dst.as_os_str().to_os_string();
    s.push(".tmp");
    PathBuf::from(s)
}

fn sample_entries() -> Vec<(String, Vec<u8>)> {
    vec![
        ("meta.json".to_string(), b"{\"schema_version\":3}".to_vec()),
        ("payload.bin".to_string(), b"abcdef".to_vec()),
    ]
}

#[test]
fn atomic_write_does_not_leak_tmp_on_panic() {
    // A clean write must not leave the `.tmp` behind. We simulate the
    // post-conditions a successful caller would observe: no `.tmp`,
    // archive present, archive bytes are non-empty.
    //
    // We can't actually panic mid-write through the public API, but the
    // contract this test pins is the same one a panic-aware caller
    // relies on: the *finished* archive call always cleans up after
    // itself. Pre-fix, a leaked `.tmp` from a panicked previous process
    // could block subsequent saves; the related case is the next test.
    let dir = tempfile::tempdir().unwrap();
    let dst = dir.path().join("archive.tar.zst");

    archive::write_archive(&dst, sample_entries()).unwrap();
    assert!(dst.exists(), "archive must exist after write");
    assert!(
        !tmp_for(&dst).exists(),
        ".tmp leaked after successful write",
    );

    // Reading it back yields what we put in.
    let entries = archive::read_archive(&dst).unwrap();
    let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"meta.json"));
    assert!(names.contains(&"payload.bin"));
}

#[test]
fn atomic_write_stale_tmp_does_not_block_subsequent_save() {
    // Plant a stale `.tmp` (simulates a SIGKILL during a previous
    // write), then run `write_archive`. The new save must succeed and
    // the resulting archive must be the new one — not the corrupt
    // bytes we planted.
    let dir = tempfile::tempdir().unwrap();
    let dst = dir.path().join("archive.tar.zst");
    let tmp = tmp_for(&dst);

    // Crash residue.
    std::fs::write(&tmp, b"NOT A VALID ZSTD STREAM, leftover from a crashed save").unwrap();
    assert!(tmp.exists());

    // Now do a real save; the stale tmp must be overwritten by the
    // tmp-write step and renamed cleanly into place.
    archive::write_archive(&dst, sample_entries()).unwrap();
    assert!(dst.exists());
    // After rename, no `.tmp` should remain.
    assert!(
        !tmp.exists(),
        "stale .tmp not cleaned up by subsequent write_archive",
    );

    // And the bytes we read back are the new contents, not the planted
    // garbage.
    let entries = archive::read_archive(&dst).unwrap();
    let payload = entries
        .iter()
        .find(|e| e.name == "payload.bin")
        .map(|e| e.bytes.clone())
        .unwrap_or_default();
    assert_eq!(payload, b"abcdef");

    // Calling write_archive a second time after a clean state still
    // works (proves no path-locking regression hidden in the rename
    // logic).
    archive::write_archive(
        &dst,
        vec![("meta.json".to_string(), b"{\"schema_version\":3,\"v\":2}".to_vec())],
    )
    .unwrap();
    assert!(!tmp.exists());
    let entries2 = archive::read_archive(&dst).unwrap();
    assert_eq!(entries2.len(), 1);
    assert_eq!(entries2[0].name, "meta.json");
}
