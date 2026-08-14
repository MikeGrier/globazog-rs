// Copyright (c) 2026 Mike Grier

//! Platform layer (D-54). The **completion-based enumeration abstraction** (D-5)
//! and the native OS backends (Windows overlapped `NtQueryDirectoryFile` + IOCP,
//! Linux `getdents64` / io_uring — D-4, D-6) are sequenced follow-ups (see
//! CHECKLIST.md M5). This module currently provides the **portable reference
//! backend** ([`enumerate_dir`], safe `std::fs`) that satisfies the enumeration
//! contract and unblocks the ring/engine, plus the [`signal`] waitable primitive.

pub mod signal;

use crate::predicate::{EntryMeta, EntryType};
use crate::syntax::{decode, CodePoint};
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

/// Enumerate one directory's entries with inline metadata (the portable backend).
/// Symlink-aware: entry metadata is read without following symlinks.
pub fn enumerate_dir(path: &Path) -> io::Result<Vec<DirEntry>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        let md = entry.metadata()?; // does not traverse symlinks
        let (attributes, is_reparse, file_id, ctime) = platform_extra(&md, &ft);
        let entry_type = if ft.is_dir() {
            EntryType::Dir
        } else if ft.is_file() {
            EntryType::File
        } else {
            EntryType::Other
        };
        out.push(DirEntry {
            name: decode_name(&entry.file_name()),
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
        });
    }
    Ok(out)
}

fn nanos(t: io::Result<SystemTime>) -> i64 {
    t.ok()
        .and_then(|st| st.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
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
