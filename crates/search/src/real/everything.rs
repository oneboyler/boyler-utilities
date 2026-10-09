//! Everything (voidtools) through its own IPC window messages - the protocol its SDK (`Everything.c`, MIT) speaks, so no
//! `Everything64.dll` has to be shipped: find Everything's hidden `EVERYTHING_TASKBAR_NOTIFICATION` window, send it a
//! `WM_COPYDATA` query (`EVERYTHING_IPC_QUERY2`), and read the result list it sends back to our own message window.
//!
//! The layout below is `everything_ipc.h` of the SDK 1.4. The tests prove our side against a stand-in window built the same
//! way (tests/everything_ipc.rs). Order 043 ran it against the real Everything 1.4.1.1032 (a scratch instance on a copy of
//! the app's index, `search-show --instance`): version, index loaded, NTFS drives and queries answered (e.g. 1098 matches
//! in 28 ms). While an index loads it answers the version at once and "not loaded" until done (1.4 - 2.2 s for 104 MB);
//! it tells no file count or percent meanwhile.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{FILETIME, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    ChangeWindowMessageFilterEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, FindWindowW, MsgWaitForMultipleObjects, PeekMessageW,
    RegisterClassW, SendMessageTimeoutW, TranslateMessage, HWND_MESSAGE, MSG, MSGFLT_ALLOW, PM_REMOVE, QS_ALLINPUT, SMTO_ABORTIFHUNG,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_COPYDATA, WM_USER, WNDCLASSW,
};

use super::{stamp_from_filetime, wide};
use crate::error::{Result, SearchError};
use crate::os::{FileQuery, Hit, Hits};

/// `EVERYTHING_IPC_WNDCLASS`: the window of the default instance; a named instance adds `_(<name>)`.
pub const WNDCLASS: &str = "EVERYTHING_TASKBAR_NOTIFICATION";
/// `EVERYTHING_WM_IPC`
const WM_IPC: u32 = WM_USER;
/// `EVERYTHING_IPC_GET_MAJOR_VERSION`
const IPC_GET_MAJOR_VERSION: usize = 0;
/// `EVERYTHING_IPC_IS_DB_LOADED`
const IPC_IS_DB_LOADED: usize = 401;
/// `EVERYTHING_IPC_COPYDATA_QUERY2W`
const COPYDATA_QUERY2W: usize = 18;
/// Our reply's `dwData` (any value; Everything sends it back).
const REPLY_ID: u32 = 0x4255_5352;

// `EVERYTHING_IPC_QUERY2_REQUEST_*` (the order of the fields in an item's data)
const REQUEST_NAME: u32 = 0x1;
const REQUEST_PATH: u32 = 0x2;
const REQUEST_SIZE: u32 = 0x10;
const REQUEST_DATE_MODIFIED: u32 = 0x40;
/// `EVERYTHING_IPC_SORT_NAME_ASCENDING`
const SORT_NAME_ASCENDING: u32 = 1;
/// `EVERYTHING_IPC_FOLDER` (an item's flags)
const ITEM_FOLDER: u32 = 0x1;

/// The window class of an instance: None = the default one (a copy the user runs).
pub fn class_of(instance: Option<&str>) -> String {
    match instance {
        Some(i) => format!("{WNDCLASS}_({i})"),
        None => WNDCLASS.to_string(),
    }
}

/// Everything's IPC window of that class, if one runs.
pub fn find(class: &str) -> Option<HWND> {
    let c = wide(class);
    unsafe { FindWindowW(PCWSTR(c.as_ptr()), PCWSTR::null()) }.ok().filter(|h| !h.is_invalid())
}

fn ask(hwnd: HWND, what: usize) -> Option<usize> {
    let mut out = 0usize;
    let r = unsafe { SendMessageTimeoutW(hwnd, WM_IPC, WPARAM(what), LPARAM(0), SMTO_ABORTIFHUNG, 2000, Some(&mut out)) };
    (r.0 != 0).then_some(out)
}

/// Its major version (1 for 1.4); None = it did not answer.
pub fn major_version(hwnd: HWND) -> Option<u32> {
    ask(hwnd, IPC_GET_MAJOR_VERSION).map(|v| v as u32)
}

/// Its index is loaded (else it is still making or loading its file list).
pub fn db_loaded(hwnd: HWND) -> bool {
    ask(hwnd, IPC_IS_DB_LOADED).map(|v| v != 0).unwrap_or(false)
}

/// Its full version (`EVERYTHING_IPC_GET_MAJOR_VERSION` .. `_GET_BUILD_NUMBER` = 0 .. 3), e.g. 1.4.1.1032 (`search-show`).
pub fn version(hwnd: HWND) -> Option<[usize; 4]> {
    Some([ask(hwnd, 0)?, ask(hwnd, 1)?, ask(hwnd, 2)?, ask(hwnd, 3)?])
}

/// The drive letters whose NTFS index it holds (`EVERYTHING_IPC_IS_NTFS_DRIVE_INDEXED` = 400, lParam = drive 0-25;
/// measured on a test PC: none while a saved index loads, all five once loaded) (`search-show`).
pub fn ntfs_drives(hwnd: HWND) -> String {
    (0..26u8)
        .filter(|d| {
            let mut out = 0usize;
            let r = unsafe { SendMessageTimeoutW(hwnd, WM_IPC, WPARAM(400), LPARAM(*d as isize), SMTO_ABORTIFHUNG, 2000, Some(&mut out)) };
            r.0 != 0 && out != 0
        })
        .map(|d| (b'A' + d) as char)
        .collect()
}

/// `EVERYTHING_IPC_QUERY2` + the search text (UTF-16, 0-terminated), as the bytes `WM_COPYDATA` carries.
pub fn query2_bytes(reply_hwnd: u32, search: &str, max: u32, request_flags: u32) -> Vec<u8> {
    query2_bytes_at(reply_hwnd, search, 0, max, request_flags)
}

/// [`query2_bytes`] for the page of results that starts at `offset` (Order 069: Storage reads a whole drive in pages).
pub fn query2_bytes_at(reply_hwnd: u32, search: &str, offset: u32, max: u32, request_flags: u32) -> Vec<u8> {
    let mut b = Vec::new();
    // reply_hwnd, reply_copydata_message, search_flags, offset, max_results, request_flags, sort_type
    for v in [reply_hwnd, REPLY_ID, 0, offset, max, request_flags, SORT_NAME_ASCENDING] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    for u in search.encode_utf16().chain(Some(0)) {
        b.extend_from_slice(&u.to_le_bytes());
    }
    b
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    b.get(at..at + 8).map(|s| u64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]))
}

/// One string field of an item's data: a DWORD length (characters) then the characters and a 0.
fn str_at(b: &[u8], at: usize) -> Option<(String, usize)> {
    let n = u32_at(b, at)? as usize;
    let s = b.get(at + 4..at + 4 + n * 2)?;
    let u: Vec<u16> = (0..n).map(|i| u16::from_le_bytes([s[2 * i], s[2 * i + 1]])).collect();
    Some((String::from_utf16_lossy(&u), at + 4 + (n + 1) * 2))
}

/// `EVERYTHING_IPC_LIST2` (totitems, numitems, offset, request_flags, sort_type) + `EVERYTHING_IPC_ITEM2[numitems]`
/// (flags, data_offset - from the list's start); each item's data holds the requested fields in their bit order:
/// name, path (strings), size (8 bytes), date modified (FILETIME).
pub fn parse_list2(b: &[u8]) -> Result<Hits> {
    let bad = || SearchError::Everything("a reply Everything sent could not be read".into());
    let total = u32_at(b, 0).ok_or_else(bad)? as usize;
    let n = u32_at(b, 4).ok_or_else(bad)? as usize;
    // the count comes from another process: it must fit the bytes actually sent (never a huge allocation)
    if n > b.len().saturating_sub(20) / 8 {
        return Err(bad());
    }
    let flags = u32_at(b, 12).ok_or_else(bad)?;
    let mut items = Vec::with_capacity(n);
    for i in 0..n {
        let at = 20 + i * 8;
        let item_flags = u32_at(b, at).ok_or_else(bad)?;
        let mut p = u32_at(b, at + 4).ok_or_else(bad)? as usize;
        let (mut name, mut dir, mut size, mut modified) = (String::new(), String::new(), None, None);
        if flags & REQUEST_NAME != 0 {
            let (s, q) = str_at(b, p).ok_or_else(bad)?;
            name = s;
            p = q;
        }
        if flags & REQUEST_PATH != 0 {
            let (s, q) = str_at(b, p).ok_or_else(bad)?;
            dir = s;
            p = q;
        }
        // fields between path and size that we never ask for (full path 0x4, extension 0x8): skipped if sent anyway
        for f in [0x4u32, 0x8] {
            if flags & f != 0 {
                p = str_at(b, p).ok_or_else(bad)?.1;
            }
        }
        if flags & REQUEST_SIZE != 0 {
            size = u64_at(b, p);
            p += 8;
        }
        if flags & 0x20 != 0 {
            p += 8; // date created (never asked)
        }
        if flags & REQUEST_DATE_MODIFIED != 0 {
            let v = u64_at(b, p).ok_or_else(bad)?;
            modified = stamp_from_filetime(FILETIME { dwLowDateTime: v as u32, dwHighDateTime: (v >> 32) as u32 });
        }
        // `EVERYTHING_IPC_FOLDER` 0x1, `EVERYTHING_IPC_DRIVE` 0x2 (a drive root is a folder too)
        let is_folder = item_flags & (ITEM_FOLDER | 0x2) != 0;
        let path = if dir.is_empty() { name.clone() } else { format!("{}\\{}", dir.trim_end_matches('\\'), name) };
        // a folder's size is Everything's own sum (or -1 when it keeps none): the page counts its items instead
        let size = if is_folder { None } else { size.filter(|s| *s != u64::MAX) };
        items.push(Hit { name, path, is_folder, size, modified });
    }
    Ok(Hits { items, total: Some(total) })
}

thread_local! {
    /// The reply our message window got (one query at a time per thread).
    static REPLY: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
}

unsafe extern "system" fn reply_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_COPYDATA && lp.0 != 0 {
        let cds = &*(lp.0 as *const COPYDATASTRUCT);
        if cds.dwData == REPLY_ID as usize && !cds.lpData.is_null() {
            let data = std::slice::from_raw_parts(cds.lpData as *const u8, cds.cbData as usize).to_vec();
            REPLY.with(|r| *r.borrow_mut() = Some(data));
            return LRESULT(1);
        }
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

/// Our message-only window that receives the reply (this thread's).
struct ReplyWindow(HWND);

impl ReplyWindow {
    fn new() -> Result<ReplyWindow> {
        let class = wide("BoylerUtilities.EverythingReply");
        unsafe {
            let inst = GetModuleHandleW(None).map_err(|e| super::os_err("GetModuleHandleW", e))?;
            let wc = WNDCLASSW { lpfnWndProc: Some(reply_proc), hInstance: inst.into(), lpszClassName: PCWSTR(class.as_ptr()), ..Default::default() };
            // a second registration fails with "class already exists": fine
            RegisterClassW(&wc);
            let h = CreateWindowExW(WINDOW_EX_STYLE(0), PCWSTR(class.as_ptr()), PCWSTR::null(), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, Some(inst.into()), None)
                .map_err(|e| super::os_err("CreateWindowExW", e))?;
            // an elevated app still gets the reply of an Everything that is not (UIPI); what any sender may put there is
            // bounded by `parse_list2`
            let _ = ChangeWindowMessageFilterEx(h, WM_COPYDATA, MSGFLT_ALLOW, None);
            Ok(ReplyWindow(h))
        }
    }
}

impl Drop for ReplyWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}

/// One query: names, folders, sizes and dates of at most `q.max` items, Everything's own name match.
pub fn query(everything: HWND, q: &FileQuery, timeout: Duration) -> Result<Hits> {
    query_text(everything, &q.everything_text(), q.max as u32, timeout)
}

/// The same with Everything's own search text (`search-show --raw`).
pub fn query_text(everything: HWND, text: &str, max: u32, timeout: Duration) -> Result<Hits> {
    let win = ReplyWindow::new()?;
    REPLY.with(|r| *r.borrow_mut() = None);
    let bytes = query2_bytes(win.0 .0 as usize as u32, text, max, REQUEST_NAME | REQUEST_PATH | REQUEST_SIZE | REQUEST_DATE_MODIFIED);
    let cds = COPYDATASTRUCT { dwData: COPYDATA_QUERY2W, cbData: bytes.len() as u32, lpData: bytes.as_ptr() as *mut _ };
    let mut ok = 0usize;
    let sent = unsafe { SendMessageTimeoutW(everything, WM_COPYDATA, WPARAM(win.0 .0 as usize), LPARAM(&cds as *const _ as isize), SMTO_ABORTIFHUNG, 5000, Some(&mut ok)) };
    if sent.0 == 0 || ok == 0 {
        return Err(SearchError::Everything("Everything did not take the query".into()));
    }
    // the answer comes as a WM_COPYDATA sent to our window: pump this thread's messages until it is there
    let until = Instant::now() + timeout;
    loop {
        if let Some(b) = REPLY.with(|r| r.borrow_mut().take()) {
            return parse_list2(&b);
        }
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(SearchError::Everything("no answer in time".into()));
        }
        unsafe {
            let mut m = MSG::default();
            while PeekMessageW(&mut m, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&m);
                DispatchMessageW(&m);
            }
            if REPLY.with(|r| r.borrow().is_some()) {
                continue;
            }
            MsgWaitForMultipleObjects(None, false, (left.as_millis() as u32).clamp(1, 100), QS_ALLINPUT);
        }
    }
}

// ------------------------------------------------------------------------------------------------- whole-drive lists (Order 069)

/// `EVERYTHING_IPC_QUERY2_REQUEST_ATTRIBUTES`
const REQUEST_ATTRIBUTES: u32 = 0x100;

/// One item of a list: a file or a folder with the size and attributes Everything's index holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListItem {
    pub name: String,
    /// the folder it is in (a drive root has none)
    pub dir: String,
    /// bytes (a file's size; `u64::MAX` = Everything keeps none)
    pub size: u64,
    /// Windows file attributes (0 when none came)
    pub attrs: u32,
    pub is_folder: bool,
}

/// One page of a list: how many items match in all, and this page's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListPage {
    pub total: usize,
    pub items: Vec<ListItem>,
}

/// What a list asks for: name, folder, size, attributes (nothing else is sent - the reply is as small as it can be).
pub const LIST_FIELDS: u32 = REQUEST_NAME | REQUEST_PATH | REQUEST_SIZE | REQUEST_ATTRIBUTES;

/// The reply of a [`LIST_FIELDS`] query (`parse_list2` reads the Search tab's fields; this one the list's).
pub fn parse_list(b: &[u8]) -> Result<ListPage> {
    let bad = || SearchError::Everything("a reply Everything sent could not be read".into());
    let total = u32_at(b, 0).ok_or_else(bad)? as usize;
    let n = u32_at(b, 4).ok_or_else(bad)? as usize;
    // the count comes from another process: it must fit the bytes actually sent (never a huge allocation)
    if n > b.len().saturating_sub(20) / 8 {
        return Err(bad());
    }
    let flags = u32_at(b, 12).ok_or_else(bad)?;
    let mut items = Vec::with_capacity(n);
    for i in 0..n {
        let at = 20 + i * 8;
        let item_flags = u32_at(b, at).ok_or_else(bad)?;
        let mut p = u32_at(b, at + 4).ok_or_else(bad)? as usize;
        let (mut name, mut dir, mut size, mut attrs) = (String::new(), String::new(), u64::MAX, 0u32);
        if flags & REQUEST_NAME != 0 {
            let (s, q) = str_at(b, p).ok_or_else(bad)?;
            name = s;
            p = q;
        }
        if flags & REQUEST_PATH != 0 {
            let (s, q) = str_at(b, p).ok_or_else(bad)?;
            dir = s;
            p = q;
        }
        for f in [0x4u32, 0x8] {
            if flags & f != 0 {
                p = str_at(b, p).ok_or_else(bad)?.1;
            }
        }
        if flags & REQUEST_SIZE != 0 {
            size = u64_at(b, p).ok_or_else(bad)?;
            p += 8;
        }
        // dates (created 0x20, modified 0x40, accessed 0x80): 8 bytes each, never asked
        for f in [0x20u32, 0x40, 0x80] {
            if flags & f != 0 {
                p += 8;
            }
        }
        if flags & REQUEST_ATTRIBUTES != 0 {
            attrs = u32_at(b, p).ok_or_else(bad)?;
        }
        items.push(ListItem { name, dir, size, attrs, is_folder: item_flags & (ITEM_FOLDER | 0x2) != 0 });
    }
    Ok(ListPage { total, items })
}

/// One page of a list: Everything's own search text (`C:\ file:`), the page that starts at `offset`, at most `max` items.
pub fn list_page(everything: HWND, text: &str, offset: u32, max: u32, timeout: Duration) -> Result<ListPage> {
    let b = ask_list(everything, text, offset, max, LIST_FIELDS, timeout)?;
    parse_list(&b)
}

/// Send one QUERY2 and wait for the reply bytes.
fn ask_list(everything: HWND, text: &str, offset: u32, max: u32, flags: u32, timeout: Duration) -> Result<Vec<u8>> {
    let win = ReplyWindow::new()?;
    REPLY.with(|r| *r.borrow_mut() = None);
    let bytes = query2_bytes_at(win.0 .0 as usize as u32, text, offset, max, flags);
    let cds = COPYDATASTRUCT { dwData: COPYDATA_QUERY2W, cbData: bytes.len() as u32, lpData: bytes.as_ptr() as *mut _ };
    let mut ok = 0usize;
    let sent = unsafe { SendMessageTimeoutW(everything, WM_COPYDATA, WPARAM(win.0 .0 as usize), LPARAM(&cds as *const _ as isize), SMTO_ABORTIFHUNG, 5000, Some(&mut ok)) };
    if sent.0 == 0 || ok == 0 {
        return Err(SearchError::Everything("Everything did not take the query".into()));
    }
    let until = Instant::now() + timeout;
    loop {
        if let Some(b) = REPLY.with(|r| r.borrow_mut().take()) {
            return Ok(b);
        }
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(SearchError::Everything("no answer in time".into()));
        }
        unsafe {
            let mut m = MSG::default();
            while PeekMessageW(&mut m, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&m);
                DispatchMessageW(&m);
            }
            if REPLY.with(|r| r.borrow().is_some()) {
                continue;
            }
            MsgWaitForMultipleObjects(None, false, (left.as_millis() as u32).clamp(1, 100), QS_ALLINPUT);
        }
    }
}
