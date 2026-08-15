// Copyright (c) 2026 Mike Grier

use super::{EnumPlan, enumerate_dir, nanos, read_one_entry};
use crate::predicate::EntryType;
use std::fs;
use std::time::{Duration, UNIX_EPOCH};

#[test]
fn portable_entry_failure_is_collected_not_fatal() {
    // A failing directory entry becomes a collected per-entry error rather than
    // aborting the whole directory via `?` (D-53); `read_one_entry` is that seam.
    // A full end-to-end per-entry failure is not deterministically reproducible (the
    // Windows native backend has inline metadata and never fails per-entry; a Linux
    // `statx` failure is an inherent list/stat race), so the seam is tested directly.
    let e = read_one_entry(Err(std::io::Error::from_raw_os_error(2)), EnumPlan::FULL);
    assert!(e.is_err());
}

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
    let top = enumerate_dir(root.path(), EnumPlan::FULL).unwrap().entries;
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
        let sub = enumerate_dir(&root.path().join(name), EnumPlan::FULL)
            .unwrap()
            .entries;
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
    let entries = enumerate_dir(root.path(), EnumPlan::FULL).unwrap().entries;
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

    let portable = enumerate_dir(root.path(), EnumPlan::FULL).unwrap().entries;
    let native = enumerate_dir_native(root.path(), EnumPlan::FULL)
        .unwrap()
        .entries;

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
    let files = enumerate_dir_native(&root.path().join("dir0"), EnumPlan::FULL)
        .unwrap()
        .entries;
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

    let portable = enumerate_dir(root.path(), EnumPlan::FULL).unwrap().entries;
    let native = enumerate_dir_native(root.path(), EnumPlan::FULL)
        .unwrap()
        .entries;

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
    let files = enumerate_dir_native(&root.path().join("dir0"), EnumPlan::FULL)
        .unwrap()
        .entries;
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

    let native = enumerate_dir_native(root.path(), EnumPlan::FULL)
        .unwrap()
        .entries;
    let link = native
        .iter()
        .find(|e| e.name == crate::syntax::decode::decode_bytes(b"link"))
        .expect("symlink entry present");
    assert!(link.is_reparse);
    assert_eq!(link.entry_type, EntryType::Other);
}

#[test]
fn portable_plan_without_stat_skips_metadata() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.txt"), b"hello").unwrap();

    // Names/types only: the portable backend must not stat, so size stays 0 while the
    // type (from `file_type`) is still correct (D-62).
    let plan = EnumPlan {
        want_stat: false,
        want_file_id: false,
        want_reparse_file_id: false,
    };
    let names_only = enumerate_dir(root.path(), plan).unwrap().entries;
    let f = names_only
        .iter()
        .find(|e| e.entry_type == EntryType::File)
        .unwrap();
    assert_eq!(f.size, 0);

    // With stat requested, the real size is populated.
    let full = enumerate_dir(root.path(), EnumPlan::FULL).unwrap().entries;
    let f = full
        .iter()
        .find(|e| e.entry_type == EntryType::File)
        .unwrap();
    assert_eq!(f.size, 5);
}

#[cfg(target_os = "linux")]
#[test]
fn native_linux_plan_without_stat_uses_dtype_only() {
    use super::linux::enumerate_dir_native;

    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("d")).unwrap();
    fs::write(root.path().join("f.txt"), b"hello").unwrap();

    // No stat requested: the type comes from `getdents64`'s `d_type`, and the
    // stat-tier fields (size, file id) are left unset (D-62).
    let plan = EnumPlan {
        want_stat: false,
        want_file_id: false,
        want_reparse_file_id: false,
    };
    let entries = enumerate_dir_native(root.path(), plan).unwrap().entries;
    assert!(entries.iter().any(|e| e.entry_type == EntryType::Dir));
    let f = entries
        .iter()
        .find(|e| e.entry_type == EntryType::File)
        .unwrap();
    assert_eq!(f.size, 0);
    assert_eq!(f.file_id.id, 0);
}

#[cfg(unix)]
#[test]
fn portable_reparse_only_file_id_skips_regular_entries() {
    // The reparse-only request (the engine's D-51 cycle-detection need) fetches the
    // id for a symlink but not for a regular file; the all-entries request fetches it
    // for every entry. Both are honored without `want_stat`.
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("f.txt"), b"x").unwrap();
    std::os::unix::fs::symlink("f.txt", root.path().join("link")).unwrap();

    let reparse_only = EnumPlan {
        want_stat: false,
        want_file_id: false,
        want_reparse_file_id: true,
    };
    let e = enumerate_dir(root.path(), reparse_only).unwrap().entries;
    let f = e.iter().find(|x| x.entry_type == EntryType::File).unwrap();
    let l = e.iter().find(|x| x.is_reparse).unwrap();
    assert_eq!(f.file_id.id, 0);
    assert!(l.file_id.id != 0);

    let all = EnumPlan {
        want_stat: false,
        want_file_id: true,
        want_reparse_file_id: false,
    };
    let e = enumerate_dir(root.path(), all).unwrap().entries;
    let f = e.iter().find(|x| x.entry_type == EntryType::File).unwrap();
    assert!(f.file_id.id != 0);
}

#[cfg(target_os = "linux")]
#[test]
fn native_linux_reparse_only_file_id_skips_regular_entries() {
    use super::linux::enumerate_dir_native;

    // Same distinction on the native Linux backend, where the optimization avoids one
    // `statx` per regular entry.
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("f.txt"), b"x").unwrap();
    std::os::unix::fs::symlink("f.txt", root.path().join("link")).unwrap();

    let reparse_only = EnumPlan {
        want_stat: false,
        want_file_id: false,
        want_reparse_file_id: true,
    };
    let e = enumerate_dir_native(root.path(), reparse_only)
        .unwrap()
        .entries;
    let f = e.iter().find(|x| x.entry_type == EntryType::File).unwrap();
    let l = e.iter().find(|x| x.is_reparse).unwrap();
    assert_eq!(f.file_id.id, 0);
    assert!(l.file_id.id != 0);

    let all = EnumPlan {
        want_stat: false,
        want_file_id: true,
        want_reparse_file_id: false,
    };
    let e = enumerate_dir_native(root.path(), all).unwrap().entries;
    let f = e.iter().find(|x| x.entry_type == EntryType::File).unwrap();
    assert!(f.file_id.id != 0);
}
