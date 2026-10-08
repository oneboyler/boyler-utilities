//! Directory listing and deletes for the real OS.
//!
//! `FindFirstFileExW(FindExInfoBasic, FIND_FIRST_EX_LARGE_FETCH)`: one call per folder gives every name, size and
//! attribute (no short names, bigger batches). Paths get the `\\?\` prefix so folders deeper than 260 characters work.

use super::disk::{from_wide, wide};
use std::io;
use std::path::Path;
use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::{
    DeleteFileW, FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW, FindNextFileW, GetFileAttributesW,
    RemoveDirectoryW, FIND_FIRST_EX_LARGE_FETCH, INVALID_FILE_ATTRIBUTES, WIN32_FIND_DATAW,
};

const ATTR_DIRECTORY: u32 = 0x10;
const ATTR_REPARSE: u32 = 0x400;
const ATTR_OFFLINE: u32 = 0x1000;
const ATTR_RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;
/// Junctions and symlinks are "name surrogates" (bit 29 of the reparse tag). Cloud folders (OneDrive) are reparse
/// points too but are *not* surrogates — they are walked like normal folders.
const TAG_NAME_SURROGATE: u32 = 0x2000_0000;

pub(crate) fn long_path(p: &Path) -> String {
    let s = p.to_string_lossy().replace('/', "\\");
    if s.starts_with("\\\\?\\") || s.starts_with("\\\\.\\") {
        s
    } else if let Some(unc) = s.strip_prefix("\\\\") {
        format!("\\\\?\\UNC\\{unc}")
    } else {
        format!("\\\\?\\{s}")
    }
}

fn win_err(e: windows::core::Error) -> io::Error {
    let hr = e.code().0 as u32;
    if hr & 0xFFFF_0000 == 0x8007_0000 {
        io::Error::from_raw_os_error((hr & 0xFFFF) as i32)
    } else {
        io::Error::other(e)
    }
}

pub fn read_dir(path: &Path) -> io::Result<Vec<crate::RawEntry>> {
    let mut base = long_path(path);
    if !base.ends_with('\\') {
        base.push('\\');
    }
    let pattern = wide(&format!("{base}*"));
    let mut data = WIN32_FIND_DATAW::default();
    let handle = unsafe {
        FindFirstFileExW(
            PCWSTR(pattern.as_ptr()),
            FindExInfoBasic,
            &mut data as *mut _ as *mut _,
            FindExSearchNameMatch,
            None,
            FIND_FIRST_EX_LARGE_FETCH,
        )
    };
    let handle = match handle {
        Ok(h) => h,
        Err(e) => {
            let err = win_err(e);
            // An empty drive root answers "file not found"; a missing folder has no attributes.
            if err.raw_os_error() == Some(2) {
                let w = wide(&long_path(path));
                if unsafe { GetFileAttributesW(PCWSTR(w.as_ptr())) } != INVALID_FILE_ATTRIBUTES {
                    return Ok(Vec::new());
                }
                return Err(io::Error::from(io::ErrorKind::NotFound));
            }
            return Err(err);
        }
    };
    let mut out = Vec::new();
    loop {
        let name = from_wide(&data.cFileName);
        if name != "." && name != ".." {
            let attrs = data.dwFileAttributes;
            let is_dir = attrs & ATTR_DIRECTORY != 0;
            let is_reparse = attrs & ATTR_REPARSE != 0 && data.dwReserved0 & TAG_NAME_SURROGATE != 0;
            out.push(crate::RawEntry {
                name,
                is_dir,
                size: if is_dir { 0 } else { ((data.nFileSizeHigh as u64) << 32) | data.nFileSizeLow as u64 },
                is_reparse,
                is_cloud_only: !is_dir && attrs & (ATTR_OFFLINE | ATTR_RECALL_ON_DATA_ACCESS) != 0,
            });
        }
        if unsafe { FindNextFileW(handle, &mut data) }.is_err() {
            break;
        }
    }
    unsafe {
        let _ = FindClose(handle);
    }
    Ok(out)
}

pub fn remove_file(path: &Path) -> io::Result<()> {
    let w = wide(&long_path(path));
    unsafe { DeleteFileW(PCWSTR(w.as_ptr())) }.map_err(win_err)
}

pub fn remove_dir(path: &Path) -> io::Result<()> {
    let w = wide(&long_path(path));
    unsafe { RemoveDirectoryW(PCWSTR(w.as_ptr())) }.map_err(win_err)
}
