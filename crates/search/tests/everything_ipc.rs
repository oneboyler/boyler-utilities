//! The Everything IPC layer, proven against a STAND-IN: a hidden, never shown window this test creates with the class
//! of a test-only instance (`EVERYTHING_TASKBAR_NOTIFICATION_(bu-test-<pid>)`), answering the way `everything_ipc.h`
//! says Everything does. Everything itself is not installed on this PC, so this proves our side of the contract (the
//! query bytes, the reply window, the list layout, version / index-loaded asks) - not Everything. A real Everything is
//! never asked: the layer is pointed at the stand-in's class only.
#![cfg(windows)]

use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Duration;

use bu_search::real::everything;
use bu_search::*;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

/// What the stand-in got: the search text, max, request flags.
static GOT: Mutex<Option<(String, u32, u32)>> = Mutex::new(None);
static LOADED: Mutex<bool> = Mutex::new(true);

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn le(v: u32) -> [u8; 4] {
    v.to_le_bytes()
}

fn push_str(b: &mut Vec<u8>, s: &str) {
    let u: Vec<u16> = s.encode_utf16().collect();
    b.extend_from_slice(&le(u.len() as u32));
    for c in u.iter().chain(Some(&0)) {
        b.extend_from_slice(&c.to_le_bytes());
    }
}

/// `EVERYTHING_IPC_LIST2` with a folder and a file (name, path, size, date modified).
fn list2(flags: u32) -> Vec<u8> {
    let items: [(u32, &str, &str, u64); 2] = [(1, "Clips", r"C:\Users\x\Videos", u64::MAX), (0, "ace.mp4", r"C:\Users\x\Videos\Clips", 88_298_291)];
    let head = 20 + 8 * items.len();
    let mut data = Vec::new();
    let mut offs = Vec::new();
    for (_, name, path, size) in items {
        offs.push((head + data.len()) as u32);
        push_str(&mut data, name);
        push_str(&mut data, path);
        data.extend_from_slice(&size.to_le_bytes());
        // 6 Oct 2026 19:45 UTC as a FILETIME
        data.extend_from_slice(&134_042_355_000_000_000u64.to_le_bytes());
    }
    let mut b = Vec::new();
    for v in [7u32, items.len() as u32, 0, flags, 1] {
        b.extend_from_slice(&le(v));
    }
    for (i, (f, ..)) in items.iter().enumerate() {
        b.extend_from_slice(&le(*f));
        b.extend_from_slice(&le(offs[i]));
    }
    b.extend_from_slice(&data);
    b
}

unsafe extern "system" fn stand_in(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        m if m == WM_USER => match wp.0 {
            0 => LRESULT(1),
            401 => LRESULT(*LOADED.lock().unwrap() as isize),
            _ => LRESULT(0),
        },
        WM_COPYDATA => {
            let cds = &*(lp.0 as *const COPYDATASTRUCT);
            if cds.dwData != 18 {
                return LRESULT(0);
            }
            let b = std::slice::from_raw_parts(cds.lpData as *const u8, cds.cbData as usize);
            let u = |i: usize| u32::from_le_bytes([b[i * 4], b[i * 4 + 1], b[i * 4 + 2], b[i * 4 + 3]]);
            let (reply, reply_id, max, flags) = (u(0), u(1), u(4), u(5));
            let text: Vec<u16> = (28..b.len() - 1).step_by(2).map(|i| u16::from_le_bytes([b[i], b[i + 1]])).take_while(|c| *c != 0).collect();
            *GOT.lock().unwrap() = Some((String::from_utf16_lossy(&text), max, flags));
            let out = list2(flags);
            let rc = COPYDATASTRUCT { dwData: reply_id as usize, cbData: out.len() as u32, lpData: out.as_ptr() as *mut _ };
            SendMessageW(HWND(reply as usize as *mut _), WM_COPYDATA, Some(WPARAM(hwnd.0 as usize)), Some(LPARAM(&rc as *const _ as isize)));
            LRESULT(1)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// Run the stand-in window on its own thread; returns its class.
fn start_stand_in() -> String {
    let class = format!("{}_(bu-test-{})", everything::WNDCLASS, std::process::id());
    let (tx, rx) = mpsc::channel();
    let c2 = class.clone();
    std::thread::spawn(move || unsafe {
        let cw = wide(&c2);
        let inst = GetModuleHandleW(None).unwrap();
        let wc = WNDCLASSW { lpfnWndProc: Some(stand_in), hInstance: inst.into(), lpszClassName: PCWSTR(cw.as_ptr()), ..Default::default() };
        RegisterClassW(&wc);
        // a top-level window (FindWindow sees it) that is never shown: no WS_VISIBLE, zero size, off screen
        let h = CreateWindowExW(WS_EX_TOOLWINDOW, PCWSTR(cw.as_ptr()), PCWSTR::null(), WS_POPUP, -32000, -32000, 0, 0, None, None, Some(inst.into()), None).unwrap();
        tx.send(()).unwrap();
        let mut m = MSG::default();
        while GetMessageW(&mut m, None, 0, 0).as_bool() {
            DispatchMessageW(&m);
        }
        let _ = DestroyWindow(h);
    });
    rx.recv_timeout(Duration::from_secs(5)).unwrap();
    class
}

#[test]
fn queries_go_out_and_results_come_back_the_ipc_way() {
    let class = start_stand_in();
    let os = RealOs::read_only().with_everything_class(&class);
    assert_eq!(os.everything_status(), EverythingStatus::Running { version: 1 });
    // the index still loading = "catching up"
    *LOADED.lock().unwrap() = false;
    assert_eq!(os.everything_status(), EverythingStatus::Loading { building: false });
    *LOADED.lock().unwrap() = true;

    let fq = FileQuery { words: vec!["ace".into()], folders: false, extensions: vec!["mp4".into()], max: 300 };
    let hits = os.everything_query(&fq).unwrap();
    let (text, max, flags) = GOT.lock().unwrap().clone().unwrap();
    assert_eq!(text, "file: \"ace\" ext:mp4");
    assert_eq!(max, 300);
    assert_eq!(flags, 0x1 | 0x2 | 0x10 | 0x40, "name, path, size, date modified");
    assert_eq!(hits.total, Some(7));
    assert_eq!(hits.items.len(), 2);
    let f = &hits.items[0];
    assert!(f.is_folder && f.name == "Clips" && f.path == r"C:\Users\x\Videos\Clips" && f.size.is_none(), "{f:?}");
    let a = &hits.items[1];
    assert!(!a.is_folder && a.path == r"C:\Users\x\Videos\Clips\ace.mp4" && a.size == Some(88_298_291), "{a:?}");
    assert!(a.modified.is_some());

    // through the service: files come from it, and release never stops a copy that is not ours
    let svc = SearchService::new(std::sync::Arc::new(os));
    let r = svc.search(&Query::new("ace", Filter::Files), &Cancel::new()).unwrap();
    assert_eq!(r.files_from, FilesFrom::Everything { version: 1 });
    svc.release();
    assert_eq!(svc.engine(), EverythingStatus::Running { version: 1 });
}

#[test]
fn a_missing_window_is_not_running_and_a_query_fails_cleanly() {
    let os = RealOs::read_only().with_everything_class("EVERYTHING_TASKBAR_NOTIFICATION_(bu-test-none)");
    assert_eq!(os.everything_status(), EverythingStatus::NotRunning);
    let fq = FileQuery { words: vec!["x".into()], folders: true, extensions: vec![], max: 10 };
    assert!(matches!(os.everything_query(&fq), Err(SearchError::Everything(_))));
}

#[test]
fn the_reply_layout_is_read_field_by_field() {
    let hits = everything::parse_list2(&list2(0x1 | 0x2 | 0x10 | 0x40)).unwrap();
    assert_eq!(hits.items.iter().map(|h| h.name.as_str()).collect::<Vec<_>>(), vec!["Clips", "ace.mp4"]);
    assert!(everything::parse_list2(&[1, 2, 3]).is_err(), "a cut reply is an error, not a crash");
    // a reply claiming 4 billion items in 20 bytes: refused before any allocation
    let mut huge = list2(0x1);
    huge[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(everything::parse_list2(&huge).is_err());
    let q = everything::query2_bytes(0x1234, "ab", 5, 0x13);
    assert_eq!(&q[..4], &0x1234u32.to_le_bytes());
    assert_eq!(&q[16..20], &5u32.to_le_bytes());
    assert_eq!(&q[28..], &[b'a', 0, b'b', 0, 0, 0]);
}

#[test]
fn the_installer_check_is_sha256() {
    // the empty input's well-known SHA-256: proves the CNG call and the hex form (nothing is downloaded)
    assert_eq!(bu_search::real::host::sha256_hex(b"").unwrap(), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    assert_eq!(bu_search::real::host::MSI_SHA256.len(), 64);
}
