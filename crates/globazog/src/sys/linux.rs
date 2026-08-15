// Copyright (c) 2026 Mike Grier

//! Native Linux enumeration backend (D-6, D-9, D-13). Opens a directory fd and
//! reads it with `getdents64` (via the safe [`rustix::fs::Dir`] wrapper), then
//! fills each entry's metadata with `statx` — the one syscall that exposes the
//! birth time (`btime`, D-13) and a nanosecond stat tier that `std` does not.
//! Names are shipped as raw bytes decoded reversibly (D-46). Relative-open by
//! parent fd (D-9) is a pending follow-up (M7-6); this backend currently opens the
//! supplied path directly, and the enumeration works on any directory fd.
//!
//! Unlike the Windows backend (whose directory query returns inline metadata),
//! Linux `getdents64` yields only name + `d_type` + `d_ino`, so size/timestamps
//! require a per-entry `statx` — the platform reality (D-6), not a design choice.

use super::{DirEntry, FileId};
use crate::predicate::EntryType;
use crate::syntax::decode;
use rustix::fs::{self, AtFlags, Dir, FileType, Mode, OFlags, Statx, StatxFlags};
use std::ffi::CStr;
use std::io;
use std::path::Path;

/// Enumerate one directory natively, returning entries with full stat metadata.
pub fn enumerate_dir_native(path: &Path) -> io::Result<Vec<DirEntry>> {
    let dirfd = fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )?;

    // Read all names first (getdents advances the fd offset); statx afterwards.
    let mut names: Vec<std::ffi::CString> = Vec::new();
    let dir = Dir::read_from(&dirfd)?;
    for entry in dir {
        let entry = entry?;
        let name = entry.file_name();
        if is_dot_entry(name) {
            continue;
        }
        names.push(name.to_owned());
    }

    let mask = StatxFlags::TYPE
        | StatxFlags::SIZE
        | StatxFlags::INO
        | StatxFlags::ATIME
        | StatxFlags::MTIME
        | StatxFlags::CTIME
        | StatxFlags::BTIME;

    let mut out = Vec::with_capacity(names.len());
    for name in &names {
        let st = fs::statx(&dirfd, name.as_c_str(), AtFlags::SYMLINK_NOFOLLOW, mask)?;
        out.push(make_entry(name, &st));
    }
    Ok(out)
}

fn make_entry(name: &CStr, st: &Statx) -> DirEntry {
    let file_type = FileType::from_raw_mode(u32::from(st.stx_mode));
    let (entry_type, is_reparse) = match file_type {
        FileType::Directory => (EntryType::Dir, false),
        FileType::RegularFile => (EntryType::File, false),
        FileType::Symlink => (EntryType::Other, true),
        _ => (EntryType::Other, false),
    };
    let volume = rustix::fs::makedev(st.stx_dev_major, st.stx_dev_minor);
    DirEntry {
        name: decode::decode_bytes(name.to_bytes()),
        entry_type,
        is_reparse,
        // Linux has no reparse tag; attributes are a Windows-rich bitmask (D-56).
        reparse_tag: 0,
        attributes: 0,
        size: st.stx_size,
        btime: statx_time(st, StatxFlags::BTIME, st.stx_btime),
        mtime: statx_time(st, StatxFlags::MTIME, st.stx_mtime),
        atime: statx_time(st, StatxFlags::ATIME, st.stx_atime),
        ctime: statx_time(st, StatxFlags::CTIME, st.stx_ctime),
        file_id: FileId {
            volume,
            id: u128::from(st.stx_ino),
        },
    }
}

/// Nanoseconds since the Unix epoch for a statx timestamp, or 0 when the kernel did
/// not populate that field (e.g. `btime` on a filesystem that lacks it).
fn statx_time(st: &Statx, field: StatxFlags, ts: rustix::fs::StatxTimestamp) -> i64 {
    if !StatxFlags::from_bits_truncate(st.stx_mask).contains(field) {
        return 0;
    }
    ts.tv_sec
        .saturating_mul(1_000_000_000)
        .saturating_add(i64::from(ts.tv_nsec))
}

fn is_dot_entry(name: &CStr) -> bool {
    matches!(name.to_bytes(), b"." | b"..")
}
