//! Dragging files OUT of the menu (Order 014 item 1c): a page hands the frame its files (`cx.drag_out(paths)` while the
//! button is held - an `Ev::Drag`), the frame starts Windows' own drag (the shell's data object + its default drop source)
//! after the page's event: Explorer, Discord, a mail draft take them like a drag from Explorer. Copy only.
//!
//! Order 103: the data object also carries the CLASSIC file drop (CF_HDROP, + FileNameW) - Photoshop and other older apps
//! take only that, the shell's item-list format alone is not enough for them.

use windows::core::HSTRING;
use windows::Win32::System::Com::{IBindCtx, IDataObject};
use windows::Win32::System::Ole::{IDropSource, OleInitialize, DROPEFFECT_COPY};
use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{ILCreateFromPathW, ILFree, SHCreateDataObject, SHCreateShellItemArrayFromIDLists, SHDoDragDrop, BHID_DataObject};

/// The data object for `paths` (its item ids are kept alive by the returned list: free it with `free_pidls`). Built from an
/// IShellItemArray (BHID_DataObject): that one offers the shell's item list AND the classic file drop (CF_HDROP) - the
/// plain `SHCreateDataObject(None, absolute ids)` says yes to CF_HDROP but fails the GetData (Order 103: Photoshop).
unsafe fn data_object(paths: &[String]) -> Option<(windows::core::Result<IDataObject>, Vec<*mut ITEMIDLIST>)> {
    let pidls: Vec<*mut ITEMIDLIST> = paths.iter().map(|p| ILCreateFromPathW(&HSTRING::from(p.as_str()))).filter(|p| !p.is_null()).collect();
    if pidls.is_empty() {
        return None;
    }
    let refs: Vec<*const ITEMIDLIST> = pidls.iter().map(|p| *p as *const ITEMIDLIST).collect();
    let data: windows::core::Result<IDataObject> = SHCreateShellItemArrayFromIDLists(&refs).and_then(|a| a.BindToHandler(None::<&IBindCtx>, &BHID_DataObject));
    // (no array / handler: the shell's own list-only object - Explorer and Discord take it)
    let data = data.or_else(|_| SHCreateDataObject(None, Some(&refs), None::<&IDataObject>));
    Some((data, pidls))
}

unsafe fn free_pidls(pidls: Vec<*mut ITEMIDLIST>) {
    for p in pidls {
        ILFree(Some(p as *const ITEMIDLIST));
    }
}

/// Run the drag (modal: it returns when the button comes up). True = the files were dropped somewhere.
pub fn drag_out(paths: &[String]) -> bool {
    unsafe {
        // drag and drop needs OLE on this thread (a second call only returns S_FALSE)
        let _ = OleInitialize(None);
        let Some((data, pidls)) = data_object(paths) else { return false };
        // the menu holds the mouse since the button went down; the drag takes it over
        let _ = ReleaseCapture();
        let owner = crate::MENU_HWND.with(|h| *h.borrow());
        let r = data.and_then(|d| crate::modal(|| SHDoDragDrop(if owner.0.is_null() { None } else { Some(owner) }, &d, None::<&IDropSource>, DROPEFFECT_COPY)));
        free_pidls(pidls);
        r.is_ok_and(|e| e.0 != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::core::w;
    use windows::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
    use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
    use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
    use windows::Win32::System::Ole::{ReleaseStgMedium, CF_HDROP};
    use windows::Win32::UI::Shell::DROPFILES;

    fn hdrop_format() -> FORMATETC {
        FORMATETC { cfFormat: CF_HDROP.0, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 }
    }

    /// What a drop target that only knows the classic file drop does: ask for CF_HDROP, read the paths out of it.
    fn classic_target_takes(d: &IDataObject) -> Result<Vec<String>, String> {
        unsafe {
            let f = hdrop_format();
            if d.QueryGetData(&f).is_err() {
                return Err("QueryGetData(CF_HDROP) says no".into());
            }
            let mut m = d.GetData(&f).map_err(|e| format!("GetData(CF_HDROP): {e}"))?;
            let p = GlobalLock(m.u.hGlobal) as *const u8;
            let h = &*(p as *const DROPFILES);
            assert!(h.fWide.as_bool(), "wide names");
            let mut w = p.add(h.pFiles as usize) as *const u16;
            let mut out = Vec::new();
            loop {
                let mut n = 0;
                while *w.add(n) != 0 {
                    n += 1;
                }
                if n == 0 {
                    break;
                }
                out.push(String::from_utf16_lossy(std::slice::from_raw_parts(w, n)));
                w = w.add(n + 1);
            }
            let _ = GlobalUnlock(m.u.hGlobal);
            ReleaseStgMedium(&mut m);
            Ok(out)
        }
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("bu-dragout-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_classic_file_drop_target_receives_the_dragged_files() {
        unsafe {
            let _ = OleInitialize(None);
        }
        let dir = scratch("a");
        let paths: Vec<String> = ["one.png", "two.png"].iter().map(|n| {
            let p = dir.join(n);
            std::fs::write(&p, b"x").unwrap();
            p.to_string_lossy().into_owned()
        }).collect();
        let (data, pidls) = unsafe { data_object(&paths).unwrap() };
        let data = data.unwrap();
        // Explorer / Discord take the shell's item list: it stays offered
        let idlist = FORMATETC { cfFormat: unsafe { RegisterClipboardFormatW(w!("Shell IDList Array")) } as u16, ..hdrop_format() };
        assert!(unsafe { data.QueryGetData(&idlist) }.is_ok(), "the shell item list is still there");
        let got = classic_target_takes(&data);
        unsafe { free_pidls(pidls) };
        let _ = std::fs::remove_dir_all(&dir);
        let mut got = got.unwrap();
        got.sort();
        assert_eq!(got, paths);
    }
}
