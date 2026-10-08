//! Open / reveal / open with / clipboard through the shell. Only reached through `RealOs` (never on the read-only layer).

use windows::core::{PCWSTR, w};
use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use windows::Win32::Foundation::GlobalFree;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::UI::Shell::{
    SHOpenFolderAndSelectItems, SHOpenWithDialog, SHParseDisplayName, ShellExecuteW, OAIF_ALLOW_REGISTRATION, OAIF_EXEC, OPENASINFO,
};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::Win32::System::Com::CoTaskMemFree;

use super::{os_err, wide, Com};
use crate::error::{Result, SearchError};
use crate::model::OpenTarget;

const CF_UNICODETEXT: u32 = 13;

pub fn open(target: &OpenTarget) -> Result<()> {
    let _com = Com::sta();
    let what = match target {
        OpenTarget::App(parsing) => format!("shell:AppsFolder\\{parsing}"),
        OpenTarget::Path(p) => p.clone(),
    };
    let w = wide(&what);
    let h = unsafe { ShellExecuteW(None, w!("open"), PCWSTR(w.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
    // ShellExecute reports success with a value above 32
    if (h.0 as isize) > 32 {
        Ok(())
    } else {
        Err(SearchError::Os { call: "ShellExecute".into(), code: h.0 as isize as u32, text: format!("could not open {what}") })
    }
}

pub fn reveal(path: &str) -> Result<()> {
    let _com = Com::sta();
    let w = wide(path.trim_end_matches('\\'));
    unsafe {
        let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
        SHParseDisplayName(PCWSTR(w.as_ptr()), None, &mut pidl, 0, None).map_err(|e| os_err("SHParseDisplayName", e))?;
        let r = SHOpenFolderAndSelectItems(pidl, None, 0);
        CoTaskMemFree(Some(pidl as *const _));
        r.map_err(|e| os_err("SHOpenFolderAndSelectItems", e))
    }
}

pub fn open_with(path: &str) -> Result<()> {
    let _com = Com::sta();
    let w = wide(path);
    let info = OPENASINFO { pcszFile: PCWSTR(w.as_ptr()), pcszClass: PCWSTR::null(), oaifInFlags: OAIF_EXEC | OAIF_ALLOW_REGISTRATION };
    unsafe { SHOpenWithDialog(None, &info).map_err(|e| os_err("SHOpenWithDialog", e)) }
}

pub fn copy_text(text: &str) -> Result<()> {
    let data = wide(text);
    let bytes = data.len() * 2;
    unsafe {
        OpenClipboard(None).map_err(|e| os_err("OpenClipboard", e))?;
        let result = (|| -> Result<()> {
            EmptyClipboard().map_err(|e| os_err("EmptyClipboard", e))?;
            let mem: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, bytes).map_err(|e| os_err("GlobalAlloc", e))?;
            let p = GlobalLock(mem) as *mut u16;
            if p.is_null() {
                let _ = GlobalFree(Some(mem));
                return Err(SearchError::Os { call: "GlobalLock".into(), code: 0, text: "could not lock memory".into() });
            }
            std::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len());
            let _ = GlobalUnlock(mem);
            // after a successful SetClipboardData the clipboard owns the memory
            match SetClipboardData(CF_UNICODETEXT, Some(HANDLE(mem.0))) {
                Ok(_) => Ok(()),
                Err(e) => {
                    let _ = GlobalFree(Some(mem));
                    Err(os_err("SetClipboardData", e))
                }
            }
        })();
        let _ = CloseClipboard();
        result
    }
}
