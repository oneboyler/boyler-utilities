//! Windows Search's own index. No admin, no extra memory of ours: the `WSearch` service keeps the index.
//! * `service_status` — is the service there / running / disabled (the Search tab says so when it is off).
//! * `scope` — which folders the index covers, from its crawl-scope rules in the registry (read-only).
//! * `query` — OLE DB (`Search.CollatorDSO`, `SELECT ... FROM SystemIndex`), names only.

use std::ffi::c_void;

use windows::core::{Interface, GUID, PCWSTR, PWSTR};
use windows::Win32::Foundation::{ERROR_SERVICE_DOES_NOT_EXIST, SYSTEMTIME};
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_INPROC_SERVER};
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY, REG_DWORD, REG_SZ,
    REG_VALUE_TYPE,
};
use windows::Win32::System::Search::{
    IAccessor, ICommand, ICommandText, IDBCreateCommand, IDBCreateSession, IDBInitialize, IRowset, DBBINDING, DBTIMESTAMP, HACCESSOR,
};
use windows::Win32::System::Services::{
    CloseServiceHandle, OpenSCManagerW, OpenServiceW, QueryServiceConfigW, QueryServiceStatus, QUERY_SERVICE_CONFIGW, SC_HANDLE,
    SC_MANAGER_CONNECT, SERVICE_AUTO_START, SERVICE_DISABLED, SERVICE_QUERY_CONFIG, SERVICE_QUERY_STATUS, SERVICE_RUNNING, SERVICE_STATUS,
};

use super::{os_err, stamp_from_systemtime_utc, wide, Com};
use crate::error::{Result, SearchError};
use crate::os::{FileQuery, Hit, Hits, IndexScope, WsStatus};

/// `CLSID_CollatorDataSource` — the Windows Search OLE DB data source.
const CLSID_COLLATOR_DATA_SOURCE: GUID = GUID::from_u128(0x9E175B8B_F52A_11D8_B9A5_505054503030);
/// `DBGUID_DEFAULT` — the provider's default SQL dialect.
const DBGUID_DEFAULT: GUID = GUID::from_u128(0xC8B521FB_5CF3_11CE_ADE5_00AA0044773D);

const DBPART_VALUE: u32 = 1;
const DBPART_LENGTH: u32 = 2;
const DBPART_STATUS: u32 = 4;
const DBTYPE_UI8: u16 = 21;
const DBTYPE_WSTR: u16 = 130;
const DBTYPE_DBTIMESTAMP: u16 = 135;
const DBACCESSOR_ROWDATA: u32 = 2;
const DBSTATUS_S_OK: u32 = 0;

// ------------------------------------------------------------------------------------------------ the service

pub fn service_status() -> WsStatus {
    unsafe {
        let Ok(scm) = OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT) else { return WsStatus::Stopped };
        let name = wide("WSearch");
        let svc = match OpenServiceW(scm, PCWSTR(name.as_ptr()), SERVICE_QUERY_STATUS | SERVICE_QUERY_CONFIG) {
            Ok(s) => s,
            Err(e) => {
                let _ = CloseServiceHandle(scm);
                return if e.code() == ERROR_SERVICE_DOES_NOT_EXIST.to_hresult() { WsStatus::Missing } else { WsStatus::Stopped };
            }
        };
        let mut st = SERVICE_STATUS::default();
        let running = QueryServiceStatus(svc, &mut st).is_ok() && st.dwCurrentState == SERVICE_RUNNING;
        let disabled = start_type(svc) == Some(SERVICE_DISABLED.0);
        let _ = (CloseServiceHandle(svc), CloseServiceHandle(scm));
        let _ = SERVICE_AUTO_START;
        if running {
            WsStatus::Running
        } else if disabled {
            WsStatus::Disabled
        } else {
            WsStatus::Stopped
        }
    }
}

unsafe fn start_type(svc: SC_HANDLE) -> Option<u32> {
    let mut needed = 0u32;
    let _ = QueryServiceConfigW(svc, None, 0, &mut needed); // asks for the size
    if needed == 0 {
        return None;
    }
    // u64 elements keep the buffer 8-byte aligned for QUERY_SERVICE_CONFIGW
    let mut buf = vec![0u64; (needed as usize).div_ceil(8)];
    let cfg = buf.as_mut_ptr() as *mut QUERY_SERVICE_CONFIGW;
    QueryServiceConfigW(svc, Some(cfg), needed, &mut needed).ok()?;
    Some((*cfg).dwStartType.0)
}

// ------------------------------------------------------------------------------------------------ the scope

const SCOPE_KEY: &str = r"SOFTWARE\Microsoft\Windows Search\CrawlScopeManager\Windows\SystemIndex";

/// `file:///C:\[728cd639-...]\Users\` → `C:\Users\`; `file:///*\$RECYCLE.BIN\` → `*\$RECYCLE.BIN\`. Other schemes → `None`.
pub fn parse_rule_url(url: &str) -> Option<String> {
    let rest = url.strip_prefix("file:///")?;
    // drop the "[guid]\" volume marker
    let cleaned = match (rest.find('['), rest.find(']')) {
        (Some(a), Some(b)) if b > a => format!("{}{}", &rest[..a], rest[b + 1..].trim_start_matches('\\')),
        _ => rest.to_string(),
    };
    Some(cleaned)
}

pub fn scope() -> IndexScope {
    let mut scope = IndexScope::default();
    for sub in ["WorkingSetRules", "DefaultRules"] {
        let rules = read_rules(&format!(r"{SCOPE_KEY}\{sub}"));
        if rules.is_empty() {
            continue;
        }
        for (url, include) in rules {
            let Some(p) = parse_rule_url(&url) else { continue };
            if include {
                scope.included.push(p);
            } else {
                scope.excluded.push(p);
            }
        }
        break;
    }
    scope
}

fn read_rules(path: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    unsafe {
        let w = wide(path);
        let mut key = HKEY::default();
        if RegOpenKeyExW(HKEY_LOCAL_MACHINE, PCWSTR(w.as_ptr()), None, KEY_READ | KEY_WOW64_64KEY, &mut key).is_err() {
            return out;
        }
        let mut i = 0u32;
        loop {
            let mut name = [0u16; 256];
            let mut len = name.len() as u32;
            if RegEnumKeyExW(key, i, Some(PWSTR(name.as_mut_ptr())), &mut len, None, None, None, None).is_err() {
                break;
            }
            i += 1;
            let sub = String::from_utf16_lossy(&name[..len as usize]);
            let mut sk = HKEY::default();
            let sw = wide(&format!(r"{path}\{sub}"));
            if RegOpenKeyExW(HKEY_LOCAL_MACHINE, PCWSTR(sw.as_ptr()), None, KEY_READ | KEY_WOW64_64KEY, &mut sk).is_err() {
                continue;
            }
            let url = reg_string(sk, "URL");
            let include = reg_dword(sk, "Include");
            let _ = RegCloseKey(sk);
            if let (Some(u), Some(inc)) = (url, include) {
                out.push((u, inc != 0));
            }
        }
        let _ = RegCloseKey(key);
    }
    out
}

unsafe fn reg_string(key: HKEY, name: &str) -> Option<String> {
    let n = wide(name);
    let mut ty = REG_VALUE_TYPE(0);
    let mut size = 0u32;
    RegQueryValueExW(key, PCWSTR(n.as_ptr()), None, Some(&mut ty), None, Some(&mut size)).ok().ok()?;
    if ty != REG_SZ || size == 0 {
        return None;
    }
    let mut buf = vec![0u16; (size as usize).div_ceil(2) + 1];
    RegQueryValueExW(key, PCWSTR(n.as_ptr()), None, Some(&mut ty), Some(buf.as_mut_ptr() as *mut u8), Some(&mut size)).ok().ok()?;
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

unsafe fn reg_dword(key: HKEY, name: &str) -> Option<u32> {
    let n = wide(name);
    let mut ty = REG_VALUE_TYPE(0);
    let mut v = 0u32;
    let mut size = 4u32;
    RegQueryValueExW(key, PCWSTR(n.as_ptr()), None, Some(&mut ty), Some(&mut v as *mut u32 as *mut u8), Some(&mut size)).ok().ok()?;
    (ty == REG_DWORD).then_some(v)
}

// ------------------------------------------------------------------------------------------------ the query

/// One row as the provider writes it (every column has a status word so NULL / truncation are told apart from values).
#[repr(C)]
struct RowBuf {
    name_status: u32,
    name_len: usize,
    name: [u16; 260],
    path_status: u32,
    path_len: usize,
    path: [u16; 1024],
    size_status: u32,
    size: u64,
    date_status: u32,
    date: DBTIMESTAMP,
}

fn str_binding(ordinal: usize, status: usize, len: usize, value: usize, bytes: usize) -> DBBINDING {
    let mut b: DBBINDING = unsafe { std::mem::zeroed() };
    b.iOrdinal = ordinal;
    b.obStatus = status;
    b.obLength = len;
    b.obValue = value;
    b.dwPart = DBPART_VALUE | DBPART_LENGTH | DBPART_STATUS;
    b.cbMaxLen = bytes;
    b.wType = DBTYPE_WSTR;
    b
}

fn fixed_binding(ordinal: usize, status: usize, value: usize, ty: u16) -> DBBINDING {
    let mut b: DBBINDING = unsafe { std::mem::zeroed() };
    b.iOrdinal = ordinal;
    b.obStatus = status;
    b.obValue = value;
    b.dwPart = DBPART_VALUE | DBPART_STATUS;
    b.wType = ty;
    b
}

fn text(buf: &[u16], len_bytes: usize) -> String {
    let n = (len_bytes / 2).min(buf.len());
    let end = buf[..n].iter().position(|&c| c == 0).unwrap_or(n);
    String::from_utf16_lossy(&buf[..end])
}

pub fn query(q: &FileQuery) -> Result<Hits> {
    let _com = Com::sta();
    let sql = q.windows_search_sql();
    unsafe {
        let init: IDBInitialize =
            CoCreateInstance(&CLSID_COLLATOR_DATA_SOURCE, None, CLSCTX_INPROC_SERVER).map_err(|e| os_err("Windows Search data source", e))?;
        init.Initialize().map_err(|e| os_err("Windows Search connect", e))?;
        let result = run(&init, &sql, q.max, q.folders);
        let _ = init.Uninitialize();
        result
    }
}

unsafe fn run(init: &IDBInitialize, sql: &str, max: usize, folders: bool) -> Result<Hits> {
    let create: IDBCreateSession = init.cast().map_err(|e| os_err("IDBCreateSession", e))?;
    let session = create.CreateSession(None, &IDBCreateCommand::IID).map_err(|e| os_err("CreateSession", e))?;
    let cc: IDBCreateCommand = session.cast().map_err(|e| os_err("IDBCreateCommand", e))?;
    let cmd_unk = cc.CreateCommand(None, &ICommandText::IID).map_err(|e| os_err("CreateCommand", e))?;
    let ct: ICommandText = cmd_unk.cast().map_err(|e| os_err("ICommandText", e))?;
    let w = wide(sql);
    ct.SetCommandText(&DBGUID_DEFAULT, PCWSTR(w.as_ptr())).map_err(|e| os_err("SetCommandText", e))?;
    let cmd: ICommand = ct.cast().map_err(|e| os_err("ICommand", e))?;
    let mut rowset_unk = None;
    cmd.Execute(None, &IRowset::IID, None, None, Some(&mut rowset_unk)).map_err(|e| os_err("Windows Search query", e))?;
    let rowset: IRowset = rowset_unk.ok_or_else(|| SearchError::Os { call: "Windows Search query".into(), code: 0, text: "no rowset".into() })?
        .cast()
        .map_err(|e| os_err("IRowset", e))?;
    let acc: IAccessor = rowset.cast().map_err(|e| os_err("IAccessor", e))?;

    use std::mem::offset_of;
    let bindings = [
        str_binding(1, offset_of!(RowBuf, name_status), offset_of!(RowBuf, name_len), offset_of!(RowBuf, name), 260 * 2),
        str_binding(2, offset_of!(RowBuf, path_status), offset_of!(RowBuf, path_len), offset_of!(RowBuf, path), 1024 * 2),
        fixed_binding(3, offset_of!(RowBuf, size_status), offset_of!(RowBuf, size), DBTYPE_UI8),
        fixed_binding(4, offset_of!(RowBuf, date_status), offset_of!(RowBuf, date), DBTYPE_DBTIMESTAMP),
    ];
    let mut h = HACCESSOR::default();
    acc.CreateAccessor(DBACCESSOR_ROWDATA, bindings.len(), bindings.as_ptr(), size_of::<RowBuf>(), &mut h, None)
        .map_err(|e| os_err("CreateAccessor", e))?;

    let items = fetch_rows(&rowset, h, max, folders);
    let _ = acc.ReleaseAccessor(h, None);
    Ok(Hits { items, total: None })
}

/// Rows fetched per `GetNextRows` call.
const BATCH: usize = 64;

/// Read the rows of an open rowset with the accessor `h`, at most about `max`, releasing every row handle.
///
/// `IRowset::GetNextRows(…, cRows, &cRowsObtained, HROW **prghRows)`: if `*prghRows` is not NULL the provider writes the handles into
/// the caller's array; if it is NULL the provider ALLOCATES an array (the caller frees it with `CoTaskMemFree`) and writes its address
/// there. The windows-rs wrapper cannot express "my own array of cRows handles" (it passes the slice itself as `HROW **` and its length
/// as `cRows`), so the vtable is called directly with a buffer of ours.
unsafe fn fetch_rows(rowset: &IRowset, h: HACCESSOR, max: usize, folders: bool) -> Vec<Hit> {
    let mut items = Vec::new();
    let mut handles = [0usize; BATCH];
    loop {
        let own: *mut usize = handles.as_mut_ptr();
        let mut p: *mut usize = own;
        let mut got = 0usize;
        let hr = (Interface::vtable(rowset).GetNextRows)(Interface::as_raw(rowset), 0, 0, BATCH as isize, &mut got, &mut p);
        if hr.is_err() || got == 0 || p.is_null() {
            break;
        }
        let list = std::slice::from_raw_parts(p, got);
        for &hrow in list {
            let mut buf: RowBuf = std::mem::zeroed();
            if rowset.GetData(hrow, h, &mut buf as *mut RowBuf as *mut c_void).is_ok()
                && buf.name_status == DBSTATUS_S_OK
                && buf.path_status == DBSTATUS_S_OK
            {
                let utc = (buf.date_status == DBSTATUS_S_OK).then(|| SYSTEMTIME {
                    wYear: buf.date.year as u16,
                    wMonth: buf.date.month,
                    wDay: buf.date.day,
                    wHour: buf.date.hour,
                    wMinute: buf.date.minute,
                    wSecond: buf.date.second,
                    ..Default::default()
                });
                items.push(Hit {
                    name: text(&buf.name, buf.name_len),
                    path: text(&buf.path, buf.path_len),
                    is_folder: folders,
                    size: (buf.size_status == DBSTATUS_S_OK).then_some(buf.size),
                    modified: utc.and_then(|u| stamp_from_systemtime_utc(&u)),
                });
            }
        }
        let _ = rowset.ReleaseRows(got, p, std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut());
        if p != own {
            CoTaskMemFree(Some(p as *const c_void)); // the provider allocated its own array
        }
        if items.len() >= max {
            break;
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use windows::core::implement;
    use windows::Win32::System::Com::CoTaskMemAlloc;
    use windows::Win32::System::Search::{IRowset_Impl, DB_E_BADROWHANDLE};

    /// A pretend provider that follows the OLE DB contract for `GetNextRows`: with a non-NULL `*prghRows` it fills the CALLER's array;
    /// with NULL it allocates one itself (and the caller must free it). Row handles are 1-based, `GetData` refuses a handle that is not
    /// a live row, `ReleaseRows` refuses a handle twice. So the old code (a NULL array, then the allocated array's address read as a
    /// handle) gets no rows from it, and a leak of handles is visible in `live`.
    #[implement(IRowset)]
    struct FakeRowset {
        rows: Vec<(String, String, u64)>,
        next: Cell<usize>,
        live: Rc<RefCell<Vec<usize>>>,
        allocate_own_array: bool,
    }

    impl IRowset_Impl for FakeRowset_Impl {
        fn AddRefRows(&self, _c: usize, _h: *const usize, _rc: *mut u32, _st: *mut u32) -> windows::core::Result<()> {
            Ok(())
        }
        fn GetData(&self, hrow: usize, _acc: HACCESSOR, pdata: *mut c_void) -> windows::core::Result<()> {
            if !self.live.borrow().contains(&hrow) {
                return Err(DB_E_BADROWHANDLE.into());
            }
            let (name, path, size) = &self.rows[hrow - 1];
            let mut buf: RowBuf = unsafe { std::mem::zeroed() };
            for (dst, src) in buf.name.iter_mut().zip(name.encode_utf16()) {
                *dst = src;
            }
            for (dst, src) in buf.path.iter_mut().zip(path.encode_utf16()) {
                *dst = src;
            }
            buf.name_len = name.encode_utf16().count() * 2;
            buf.path_len = path.encode_utf16().count() * 2;
            buf.size = *size;
            // name_status / path_status / size_status = 0 = DBSTATUS_S_OK; the date is NULL (status 3)
            buf.date_status = 3;
            unsafe { std::ptr::write(pdata as *mut RowBuf, buf) };
            Ok(())
        }
        fn GetNextRows(&self, _res: usize, _off: isize, crows: isize, got: *mut usize, prghrows: *mut *mut usize) -> windows::core::Result<()> {
            let start = self.next.get();
            let n = (crows as usize).min(self.rows.len() - start);
            unsafe {
                if (*prghrows).is_null() {
                    // the provider allocates the array
                    assert!(self.allocate_own_array);
                    *prghrows = CoTaskMemAlloc(n.max(1) * size_of::<usize>()) as *mut usize;
                }
                for i in 0..n {
                    *(*prghrows).add(i) = start + i + 1;
                }
                *got = n;
            }
            self.live.borrow_mut().extend((start + 1)..=(start + n));
            self.next.set(start + n);
            Ok(())
        }
        fn ReleaseRows(&self, crows: usize, rghrows: *const usize, _o: *const u32, _rc: *mut u32, _st: *mut u32) -> windows::core::Result<()> {
            for i in 0..crows {
                let h = unsafe { *rghrows.add(i) };
                let mut live = self.live.borrow_mut();
                match live.iter().position(|x| *x == h) {
                    Some(at) => {
                        live.remove(at);
                    }
                    None => return Err(DB_E_BADROWHANDLE.into()),
                }
            }
            Ok(())
        }
        fn RestartPosition(&self, _res: usize) -> windows::core::Result<()> {
            self.next.set(0);
            Ok(())
        }
    }

    fn rowset(n: usize, allocate_own_array: bool) -> (IRowset, Rc<RefCell<Vec<usize>>>) {
        let rows = (0..n).map(|i| (format!("file{i}.txt"), format!(r"C:\x\file{i}.txt"), 100 + i as u64)).collect();
        let live = Rc::new(RefCell::new(Vec::new()));
        (FakeRowset { rows, next: Cell::new(0), live: live.clone(), allocate_own_array }.into(), live)
    }

    /// 150 rows = three batches of 64: every row comes back with its own data and no row handle is left unreleased.
    #[test]
    fn rows_come_back_in_batches_and_every_handle_is_released() {
        let (rs, live) = rowset(150, false);
        let items = unsafe { fetch_rows(&rs, HACCESSOR(1), 1000, false) };
        assert_eq!(items.len(), 150);
        assert_eq!(items[0].name, "file0.txt");
        assert_eq!(items[0].path, r"C:\x\file0.txt");
        assert_eq!(items[0].size, Some(100));
        assert_eq!(items[0].modified, None);
        assert_eq!(items[149].name, "file149.txt");
        assert!(live.borrow().is_empty(), "every row handle was released: {:?}", live.borrow());
    }

    #[test]
    fn the_wanted_count_stops_the_reading_after_a_batch() {
        let (rs, live) = rowset(500, false);
        let items = unsafe { fetch_rows(&rs, HACCESSOR(1), 100, true) };
        assert_eq!(items.len(), 128, "two whole batches of 64 are read, then it stops");
        assert!(items.iter().all(|i| i.is_folder));
        assert!(live.borrow().is_empty());
    }

    /// The negative control: the OLD call (the windows-rs wrapper with an array of NULLs) makes a provider allocate its own handle
    /// array and write ITS ADDRESS into the first slot, which is not a row handle. This fake shows exactly that, which is why
    /// `fetch_rows` calls the vtable with a buffer of its own.
    #[test]
    fn the_old_call_would_have_read_an_address_as_a_row_handle() {
        let (rs, _live) = rowset(5, true);
        let mut rows = [std::ptr::null_mut::<usize>(); BATCH];
        let mut got = 0usize;
        unsafe { rs.GetNextRows(0, 0, &mut got, &mut rows).unwrap() };
        assert_eq!(got, 5);
        let first = rows[0];
        assert!(rows[1..got].iter().all(|p| p.is_null()), "only slot 0 was written: the provider's own array address");
        assert!(unsafe { rs.GetData(first as usize, HACCESSOR(1), &mut std::mem::MaybeUninit::<RowBuf>::uninit() as *mut _ as *mut c_void) }.is_err());
        unsafe { CoTaskMemFree(Some(first as *const c_void)) };
    }

    #[test]
    fn an_empty_result_is_empty() {
        let (rs, _live) = rowset(0, false);
        assert!(unsafe { fetch_rows(&rs, HACCESSOR(1), 1000, false) }.is_empty());
    }
}
