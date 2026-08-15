// Copyright (c) 2026 Mike Grier

//! Platform layer (D-54). [`enumerate`] dispatches to the native synchronous
//! backend on Windows (`GetFileInformationByHandleEx` — D-4) and Linux
//! (`getdents64` + `statx` — D-6), and falls back to the **portable reference
//! backend** ([`enumerate_dir`], safe `std::fs`) on other platforms. This module
//! also provides the [`signal`] waitable primitive. The completion-based
//! enumeration abstraction (D-5) and the overlapped/async OS paths (Windows
//! overlapped `NtQueryDirectoryFile` + IOCP, Linux io_uring) remain sequenced
//! follow-ups (see CHECKLIST.md M7-6).

pub mod signal;

#[cfg(windows)]
pub mod win;

#[cfg(target_os = "linux")]
pub mod linux;

use crate::predicate::{EntryMeta, EntryType};
use crate::syntax::{CodePoint, decode};
use std::fs;
use std::io;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(test)]
mod tests;

/// A filesystem object identity for cycle detection (D-51): volume + file id. The
/// portable backend fills this only where the platform exposes it cheaply (Unix
/// `dev`/`ino`); the native backends provide it always.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FileId {
    /// Volume identity (Unix `st_dev`; Windows volume serial — native only).
    pub volume: u64,
    /// File identity within the volume (Unix inode; Windows 128-bit id — native).
    pub id: u128,
}

/// One enumerated directory entry with its inline metadata (D-13). `name` is decoded
/// into code-point space (D-46).
#[derive(Clone, Debug)]
pub struct DirEntry {
    /// The entry's own name in code-point space.
    pub name: Vec<CodePoint>,
    /// The entry kind.
    pub entry_type: EntryType,
    /// Whether the entry is a reparse point / symlink (D-13).
    pub is_reparse: bool,
    /// The reparse tag (0 in the portable backend; native only).
    pub reparse_tag: u32,
    /// The attribute bitmask (Windows `FILE_ATTRIBUTE_*`; 0 on Unix).
    pub attributes: u32,
    /// File size in bytes.
    pub size: u64,
    /// Birth / creation time (nanoseconds since the Unix epoch; 0 if unavailable).
    pub btime: i64,
    /// Last-modification time.
    pub mtime: i64,
    /// Last-access time.
    pub atime: i64,
    /// Metadata-change time (Unix ctime; 0 on Windows in the portable backend).
    pub ctime: i64,
    /// Object identity for cycle detection.
    pub file_id: FileId,
}

impl DirEntry {
    /// Borrow this entry as an [`EntryMeta`] for predicate evaluation at `depth`.
    pub fn meta(&self, depth: u32) -> EntryMeta<'_> {
        EntryMeta {
            name: &self.name,
            depth,
            entry_type: self.entry_type,
            is_reparse: self.is_reparse,
            reparse_tag: self.reparse_tag,
            attributes: self.attributes,
            size: self.size,
            btime: self.btime,
            mtime: self.mtime,
            atime: self.atime,
            ctime: self.ctime,
        }
    }
}

/// The result of enumerating one directory (D-53): the entries that were read
/// successfully, plus any per-entry failures encountered while reading their
/// metadata. A failure on one entry never discards its siblings — the outer
/// `io::Result::Err` is reserved for a directory-open/read failure that yields no
/// usable listing at all.
pub struct DirScan {
    /// The entries read successfully, with inline metadata.
    pub entries: Vec<DirEntry>,
    /// Per-entry metadata failures (e.g. an entry removed or made unstatable between
    /// listing and stat), each carrying the failing entry's name when known (D-53).
    pub entry_errors: Vec<EntryFailure>,
}

/// A per-entry failure encountered while enumerating a directory (D-53): the OS
/// error plus the failing entry's name when it can be attributed to one. `name` is
/// `None` only for a directory-level fault that no single entry owns — a late
/// read/`getdents` error that truncates the listing after some entries were read.
pub struct EntryFailure {
    /// The failing entry's decoded name, when a specific entry can be named.
    pub name: Option<Vec<CodePoint>>,
    /// The underlying OS error.
    pub source: io::Error,
}

/// What the engine needs from each entry's metadata, so a backend can skip the stat
/// syscall when only names/types are wanted (D-62). The entry type and reparse status
/// come from the directory listing itself (`d_type`) and never require a stat.
#[derive(Clone, Copy, Debug)]
pub struct EnumPlan {
    /// Fetch the stat-tier fields (size, timestamps, attributes).
    pub want_stat: bool,
    /// Fetch *every* entry's file identity (volume + file id), honored uniformly by
    /// all backends: on Linux/portable it forces a per-entry stat; on Windows it
    /// keeps the inline id and queries the volume serial.
    pub want_file_id: bool,
    /// Fetch file identity for reparse points (symlinks / junctions) **only** — the
    /// narrower need of D-51 cycle detection, which never inspects a non-reparse
    /// entry's id. Cheaper than [`want_file_id`](Self::want_file_id): it stats just
    /// the symlinks, not every regular entry.
    pub want_reparse_file_id: bool,
}

impl EnumPlan {
    /// Fetch everything — used by callers/tests that want full metadata.
    pub const FULL: EnumPlan = EnumPlan {
        want_stat: true,
        want_file_id: true,
        want_reparse_file_id: true,
    };

    /// Whether any file identity is requested (all entries or reparse-only).
    pub fn wants_any_file_id(&self) -> bool {
        self.want_file_id || self.want_reparse_file_id
    }

    /// Whether an entry with the given reparse status needs its file identity fetched.
    pub fn wants_file_id_for(&self, is_reparse: bool) -> bool {
        self.want_file_id || (is_reparse && self.want_reparse_file_id)
    }
}

/// Enumerate one directory, dispatching to the platform's native backend where one
/// exists (Windows `NtQueryDirectoryFile`, Linux `getdents64`+`statx`) and falling
/// back to the portable [`enumerate_dir`] elsewhere. This is what the engine calls
/// (D-4, D-6), so the native inline metadata / birth-time value flows through. The
/// `plan` lets a backend skip the per-entry stat when only names/types are needed.
pub fn enumerate(path: &Path, plan: EnumPlan) -> io::Result<DirScan> {
    #[cfg(windows)]
    {
        win::enumerate_dir_native(path, plan)
    }
    #[cfg(target_os = "linux")]
    {
        linux::enumerate_dir_native(path, plan)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        enumerate_dir(path, plan)
    }
}

/// Enumerate one directory's entries with inline metadata (the portable backend).
/// Symlink-aware: entry metadata is read without following symlinks. A single
/// entry's metadata failure is collected into [`DirScan::entry_errors`] rather than
/// aborting the whole directory (D-53). The per-entry stat is skipped when `plan`
/// does not ask for stat-tier fields (D-62).
pub fn enumerate_dir(path: &Path, plan: EnumPlan) -> io::Result<DirScan> {
    let mut entries = Vec::new();
    let mut entry_errors = Vec::new();
    for entry in fs::read_dir(path)? {
        match read_one_entry(entry, plan) {
            Ok(e) => entries.push(e),
            Err(failure) => entry_errors.push(failure),
        }
    }
    Ok(DirScan {
        entries,
        entry_errors,
    })
}

/// Read one portable-backend entry, statting (lstat) only when the plan needs
/// stat-tier fields, or a file identity for a symlink that might be followed (D-62,
/// D-51). The entry type comes from the directory listing (`file_type`).
fn read_one_entry(
    entry: io::Result<fs::DirEntry>,
    plan: EnumPlan,
) -> Result<DirEntry, EntryFailure> {
    let entry = entry.map_err(|source| EntryFailure { name: None, source })?;
    let name = decode_name(&entry.file_name());
    let ft = entry.file_type().map_err(|source| EntryFailure {
        name: Some(name.clone()),
        source,
    })?;
    let entry_type = if ft.is_dir() {
        EntryType::Dir
    } else if ft.is_file() {
        EntryType::File
    } else {
        EntryType::Other
    };
    if !(plan.want_stat || plan.wants_file_id_for(ft.is_symlink())) {
        return Ok(DirEntry {
            name,
            entry_type,
            is_reparse: ft.is_symlink(),
            reparse_tag: 0,
            attributes: 0,
            size: 0,
            btime: 0,
            mtime: 0,
            atime: 0,
            ctime: 0,
            file_id: FileId { volume: 0, id: 0 },
        });
    }
    let md = entry.metadata().map_err(|source| EntryFailure {
        name: Some(name.clone()),
        source,
    })?; // does not traverse symlinks
    let (attributes, is_reparse, file_id, ctime) = platform_extra(&md, &ft);
    Ok(DirEntry {
        name,
        entry_type,
        is_reparse,
        reparse_tag: 0,
        attributes,
        size: md.len(),
        btime: nanos(md.created()),
        mtime: nanos(md.modified()),
        atime: nanos(md.accessed()),
        ctime,
        file_id,
    })
}

/// Nanoseconds since the Unix epoch, sign-preserving and saturating (0 when the
/// timestamp is unavailable). Pre-1970 times are negative; magnitudes outside the
/// `i64` range saturate rather than wrap, matching the native backends.
fn nanos(t: io::Result<SystemTime>) -> i64 {
    let Ok(st) = t else { return 0 };
    match st.duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_nanos()).unwrap_or(i64::MAX),
        Err(e) => i64::try_from(e.duration().as_nanos())
            .map(|n| -n)
            .unwrap_or(i64::MIN),
    }
}

fn decode_name(os: &std::ffi::OsStr) -> Vec<CodePoint> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let units: Vec<u16> = os.encode_wide().collect();
        decode::decode_utf16(&units)
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        decode::decode_bytes(os.as_bytes())
    }
    #[cfg(not(any(windows, unix)))]
    {
        decode::decode_str(&os.to_string_lossy())
    }
}

/// Reconstruct a native `OsString` from decoded code points — the exact reverse of
/// [`decode_name`] (D-46). The engine needs it to build a child directory's
/// physical path from the decoded entry name.
pub(crate) fn encode_os_name(cps: &[CodePoint]) -> std::ffi::OsString {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        let mut units: Vec<u16> = Vec::with_capacity(cps.len());
        for &cp in cps {
            if cp > 0xFFFF {
                let c = cp - 0x1_0000;
                units.push(0xD800 + ((c >> 10) as u16));
                units.push(0xDC00 + ((c & 0x3FF) as u16));
            } else {
                units.push(cp as u16);
            }
        }
        std::ffi::OsString::from_wide(&units)
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        let mut bytes: Vec<u8> = Vec::with_capacity(cps.len());
        for &cp in cps {
            if (0xDC80..=0xDCFF).contains(&cp) {
                bytes.push((cp - 0xDC00) as u8);
            } else if let Some(c) = char::from_u32(cp) {
                let mut buf = [0u8; 4];
                bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            } else {
                bytes.extend_from_slice("\u{FFFD}".as_bytes());
            }
        }
        std::ffi::OsString::from_vec(bytes)
    }
    #[cfg(not(any(windows, unix)))]
    {
        cps.iter()
            .filter_map(|&c| char::from_u32(c))
            .collect::<String>()
            .into()
    }
}

/// Reparse point / mount-point attribute bit (Windows).
#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;

#[cfg(windows)]
fn platform_extra(md: &fs::Metadata, ft: &fs::FileType) -> (u32, bool, FileId, i64) {
    use std::os::windows::fs::MetadataExt;
    let attrs = md.file_attributes();
    let is_reparse = attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 || ft.is_symlink();
    // Change-time and by-handle file id come from the native backend (M5-4).
    (attrs, is_reparse, FileId { volume: 0, id: 0 }, 0)
}

#[cfg(unix)]
fn platform_extra(md: &fs::Metadata, ft: &fs::FileType) -> (u32, bool, FileId, i64) {
    use std::os::unix::fs::MetadataExt;
    let ctime = md
        .ctime()
        .saturating_mul(1_000_000_000)
        .saturating_add(md.ctime_nsec());
    (
        0,
        ft.is_symlink(),
        FileId {
            volume: md.dev(),
            id: u128::from(md.ino()),
        },
        ctime,
    )
}

#[cfg(not(any(windows, unix)))]
fn platform_extra(_md: &fs::Metadata, ft: &fs::FileType) -> (u32, bool, FileId, i64) {
    (0, ft.is_symlink(), FileId { volume: 0, id: 0 }, 0)
}
