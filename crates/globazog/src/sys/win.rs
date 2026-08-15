// Copyright (c) 2026 Mike Grier

//! Native Windows enumeration backend (D-4, D-9, D-13, D-30). Opens a directory
//! handle (via `std::fs::OpenOptions` with `FILE_FLAG_BACKUP_SEMANTICS`) and
//! enumerates via `GetFileInformationByHandleEx(FileIdExtdDirectoryInfo)` — a
//! documented wrapper over `NtQueryDirectoryFile` — yielding inline attributes,
//! reparse tag, all four timestamps, size, and the 128-bit file id (D-13, D-51).
//! Relative-open by parent handle (D-9) is a pending follow-up (M7-6); this backend
//! currently opens the supplied path directly, and the enumeration works on any
//! directory handle.

use super::{DirEntry, DirScan, EnumPlan, FileId};
use crate::predicate::EntryType;
use crate::syntax::decode;
use std::io;
use std::mem::{offset_of, size_of};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;

use windows_sys::Win32::Foundation::{ERROR_NO_MORE_FILES, GetLastError, HANDLE};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_ID_128, FILE_ID_EXTD_DIR_INFO, FILE_ID_INFO, FILE_LIST_DIRECTORY, FileIdExtdDirectoryInfo,
    FileIdExtdDirectoryRestartInfo, FileIdInfo, GetFileInformationByHandleEx,
};

/// 100-ns intervals between the Windows (1601) and Unix (1970) epochs.
const FILETIME_TO_UNIX_100NS: i64 = 116_444_736_000_000_000;

/// Enumerate one directory natively, returning entries with full inline metadata.
/// The listing is inline, so there are no per-entry stat failures; the only entry
/// error possible is a late `GetFileInformationByHandleEx` read error after one or
/// more successful batches, which is surfaced in [`DirScan::entry_errors`] while the
/// already-collected entries are preserved (D-53). The stat-tier fields are inline, so
/// `plan.want_stat` is moot here; the file identity, however, needs a *separate*
/// `FileIdInfo` query, so it is fetched only when `plan.want_file_id` is set (D-62).
pub fn enumerate_dir_native(path: &Path, plan: EnumPlan) -> io::Result<DirScan> {
    let dir = open_dir(path)?;
    let raw = dir.as_raw_handle() as HANDLE;
    // The volume serial is a *separate* `FileIdInfo` query (not part of the inline
    // directory listing), so only pay it — and only risk a redirector that does not
    // support it — when a file identity was actually requested (D-62).
    let volume = if plan.want_file_id {
        volume_serial(raw)?
    } else {
        0
    };

    // A u64 buffer guarantees 8-byte alignment for the i64 fields; the API keeps
    // every record 8-aligned, so `NextEntryOffset` chaining stays aligned.
    let mut buf = vec![0u64; 8 * 1024]; // 64 KiB
    let base = buf.as_mut_ptr().cast::<u8>();
    let cap = (buf.len() * size_of::<u64>()) as u32;

    let mut out = Vec::new();
    let mut entry_errors = Vec::new();
    let mut first = true;
    loop {
        let class = if first {
            FileIdExtdDirectoryRestartInfo
        } else {
            FileIdExtdDirectoryInfo
        };
        // SAFETY: `raw` is a live directory handle; `base`/`cap` describe a valid
        // writable buffer of `cap` bytes.
        let ok = unsafe { GetFileInformationByHandleEx(raw, class, base.cast(), cap) };
        if ok == 0 {
            // SAFETY: called immediately after the failed Win32 call.
            let err = unsafe { GetLastError() };
            if err == ERROR_NO_MORE_FILES {
                break;
            }
            let io_err = io::Error::from_raw_os_error(err as i32);
            // A read error after one or more successful batches: keep the usable
            // partial listing and surface the late error; only a failure with no
            // usable listing propagates as the outer `Err` (D-53).
            if out.is_empty() {
                return Err(io_err);
            }
            entry_errors.push(io_err);
            break;
        }
        first = false;

        let mut offset = 0usize;
        loop {
            // SAFETY: the API wrote a valid `FILE_ID_EXTD_DIR_INFO` at `offset`,
            // 8-aligned within `buf`.
            let rec_ptr = unsafe { base.add(offset) }.cast::<FILE_ID_EXTD_DIR_INFO>();
            let rec = unsafe { &*rec_ptr };

            let name_units = (rec.FileNameLength as usize) / size_of::<u16>();
            let name_off = offset + offset_of!(FILE_ID_EXTD_DIR_INFO, FileName);
            // SAFETY: the name follows the fixed struct fields, `name_units` long.
            let name =
                unsafe { std::slice::from_raw_parts(base.add(name_off).cast::<u16>(), name_units) };

            if !is_dot_entry(name) {
                out.push(make_entry(rec, name, volume, plan.want_file_id));
            }

            if rec.NextEntryOffset == 0 {
                break;
            }
            offset += rec.NextEntryOffset as usize;
        }
    }
    Ok(DirScan {
        entries: out,
        entry_errors,
    })
}

fn open_dir(path: &Path) -> io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .access_mode(FILE_LIST_DIRECTORY)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
}

fn volume_serial(handle: HANDLE) -> io::Result<u64> {
    // SAFETY: a zeroed FILE_ID_INFO is a valid initial state.
    let mut info: FILE_ID_INFO = unsafe { std::mem::zeroed() };
    // SAFETY: `handle` is live; the buffer matches the requested class.
    let ok = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileIdInfo,
            (&mut info as *mut FILE_ID_INFO).cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(info.VolumeSerialNumber)
}

fn make_entry(
    rec: &FILE_ID_EXTD_DIR_INFO,
    name: &[u16],
    volume: u64,
    want_file_id: bool,
) -> DirEntry {
    let attrs = rec.FileAttributes;
    let entry_type = if attrs & FILE_ATTRIBUTE_DIRECTORY != 0 {
        EntryType::Dir
    } else {
        EntryType::File
    };
    DirEntry {
        name: decode::decode_utf16(name),
        entry_type,
        is_reparse: attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0,
        reparse_tag: rec.ReparsePointTag,
        attributes: attrs,
        size: rec.EndOfFile as u64,
        btime: filetime_to_unix_nanos(rec.CreationTime),
        mtime: filetime_to_unix_nanos(rec.LastWriteTime),
        atime: filetime_to_unix_nanos(rec.LastAccessTime),
        ctime: filetime_to_unix_nanos(rec.ChangeTime),
        // File identity is only meaningful with the volume serial (D-62); when it was
        // not requested, leave it unset so cycle detection treats it as unknown.
        file_id: if want_file_id {
            FileId {
                volume,
                id: file_id_128(&rec.FileId),
            }
        } else {
            FileId { volume: 0, id: 0 }
        },
    }
}

fn file_id_128(id: &FILE_ID_128) -> u128 {
    u128::from_le_bytes(id.Identifier)
}

fn filetime_to_unix_nanos(ft: i64) -> i64 {
    if ft == 0 {
        return 0;
    }
    (ft - FILETIME_TO_UNIX_100NS).saturating_mul(100)
}

fn is_dot_entry(name: &[u16]) -> bool {
    const DOT: u16 = b'.' as u16;
    matches!(name, [DOT] | [DOT, DOT])
}
