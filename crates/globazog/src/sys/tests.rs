// Copyright (c) 2026 Mike Grier

use super::{enumerate_dir, nanos};
use crate::predicate::EntryType;
use std::fs;
use std::time::{Duration, UNIX_EPOCH};

#[test]
fn nanos_unavailable_is_zero() {
    assert_eq!(nanos(Err(std::io::Error::other("x"))), 0);
}

#[test]
fn nanos_pre_epoch_is_negative() {
    let t = UNIX_EPOCH - Duration::from_secs(1);
    assert_eq!(nanos(Ok(t)), -1_000_000_000);
}

#[test]
fn nanos_post_epoch_is_positive() {
    let t = UNIX_EPOCH + Duration::from_secs(1);
    assert_eq!(nanos(Ok(t)), 1_000_000_000);
}

#[test]
fn nanos_far_future_saturates_instead_of_wrapping() {
    // 10^10 s ≈ year 2286: 10^19 ns exceeds i64::MAX (~9.22×10^18, ≈ year 2262), so
    // it must saturate to i64::MAX rather than wrap. (A larger value would overflow
    // the platform `SystemTime` itself on Windows, so this is the constructible edge.)
    let t = UNIX_EPOCH + Duration::from_secs(10_000_000_000);
    assert_eq!(nanos(Ok(t)), i64::MAX);
}

#[test]
fn enumerate_temp_tree() {
    let root = tempfile::tempdir().unwrap();
    let mut expected_files = 0usize;
    for i in 0..10 {
        let d = root.path().join(format!("dir{i}"));
        fs::create_dir(&d).unwrap();
        for j in 0..20 {
            fs::write(d.join(format!("file{j}.txt")), b"x").unwrap();
            expected_files += 1;
        }
    }

    // Top level: exactly the 10 directories.
    let top = enumerate_dir(root.path()).unwrap();
    let dirs = top
        .iter()
        .filter(|e| e.entry_type == EntryType::Dir)
        .count();
    assert_eq!(dirs, 10);

    // Recurse one level and count files, checking per-entry metadata.
    let mut files = 0usize;
    for e in &top {
        if e.entry_type != EntryType::Dir {
            continue;
        }
        let name: String = e.name.iter().filter_map(|&c| char::from_u32(c)).collect();
        let sub = enumerate_dir(&root.path().join(name)).unwrap();
        for s in &sub {
            if s.entry_type == EntryType::File {
                files += 1;
                assert_eq!(s.size, 1);
                assert!(!s.is_reparse);
            }
        }
    }
    assert_eq!(files, expected_files);
    assert_eq!(files, 200);
}

#[test]
fn meta_view_matches_entry() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.txt"), b"hello").unwrap();
    let entries = enumerate_dir(root.path()).unwrap();
    let e = entries
        .iter()
        .find(|e| e.entry_type == EntryType::File)
        .unwrap();
    let m = e.meta(2);
    assert_eq!(m.depth, 2);
    assert_eq!(m.size, 5);
    assert_eq!(m.name, e.name.as_slice());
}

#[cfg(windows)]
#[test]
fn native_matches_portable_and_has_file_ids() {
    use super::win::enumerate_dir_native;

    let root = tempfile::tempdir().unwrap();
    for i in 0..5 {
        let d = root.path().join(format!("dir{i}"));
        fs::create_dir(&d).unwrap();
        for j in 0..10 {
            fs::write(d.join(format!("f{j}.txt")), b"hello").unwrap();
        }
    }

    let portable = enumerate_dir(root.path()).unwrap();
    let native = enumerate_dir_native(root.path()).unwrap();

    // Same set of top-level directory names (both skip `.` / `..`).
    let mut pn: Vec<Vec<u32>> = portable.iter().map(|e| e.name.clone()).collect();
    let mut nn: Vec<Vec<u32>> = native.iter().map(|e| e.name.clone()).collect();
    pn.sort();
    nn.sort();
    assert_eq!(pn, nn);
    assert_eq!(nn.len(), 5);

    // The native backend supplies real file ids (D-51).
    assert!(
        native
            .iter()
            .all(|e| e.file_id.volume != 0 || e.file_id.id != 0)
    );

    // Native file sizes are read inline (D-13).
    let files = enumerate_dir_native(&root.path().join("dir0")).unwrap();
    let file_sizes: Vec<u64> = files
        .iter()
        .filter(|e| e.entry_type == EntryType::File)
        .map(|e| e.size)
        .collect();
    assert_eq!(file_sizes.len(), 10);
    assert!(file_sizes.iter().all(|&s| s == 5));
}

#[cfg(target_os = "linux")]
#[test]
fn native_linux_matches_portable_and_has_file_ids() {
    use super::linux::enumerate_dir_native;

    let root = tempfile::tempdir().unwrap();
    for i in 0..5 {
        let d = root.path().join(format!("dir{i}"));
        fs::create_dir(&d).unwrap();
        for j in 0..10 {
            fs::write(d.join(format!("f{j}.txt")), b"hello").unwrap();
        }
    }

    let portable = enumerate_dir(root.path()).unwrap();
    let native = enumerate_dir_native(root.path()).unwrap();

    // Same set of top-level directory names (both skip `.` / `..`).
    let mut pn: Vec<Vec<u32>> = portable.iter().map(|e| e.name.clone()).collect();
    let mut nn: Vec<Vec<u32>> = native.iter().map(|e| e.name.clone()).collect();
    pn.sort();
    nn.sort();
    assert_eq!(pn, nn);
    assert_eq!(nn.len(), 5);

    // statx supplies real (dev, ino) file ids (D-51) and matches the portable
    // backend's identity for the same objects.
    assert!(native.iter().all(|e| e.file_id.id != 0));
    let mut pids: Vec<_> = portable
        .iter()
        .map(|e| (e.name.clone(), e.file_id))
        .collect();
    let mut nids: Vec<_> = native.iter().map(|e| (e.name.clone(), e.file_id)).collect();
    pids.sort_by(|a, b| a.0.cmp(&b.0));
    nids.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(pids, nids);

    // Native file sizes and modification times come from statx (D-13).
    let files = enumerate_dir_native(&root.path().join("dir0")).unwrap();
    let regular: Vec<_> = files
        .iter()
        .filter(|e| e.entry_type == EntryType::File)
        .collect();
    assert_eq!(regular.len(), 10);
    assert!(regular.iter().all(|e| e.size == 5));
    assert!(regular.iter().all(|e| e.mtime > 0));
}

#[cfg(target_os = "linux")]
#[test]
fn native_linux_reports_symlinks_without_following() {
    use super::linux::enumerate_dir_native;

    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("target.txt"), b"payload").unwrap();
    std::os::unix::fs::symlink("target.txt", root.path().join("link")).unwrap();

    let native = enumerate_dir_native(root.path()).unwrap();
    let link = native
        .iter()
        .find(|e| e.name == crate::syntax::decode::decode_bytes(b"link"))
        .expect("symlink entry present");
    assert!(link.is_reparse);
    assert_eq!(link.entry_type, EntryType::Other);
}
