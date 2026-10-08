//! Dragging files OUT of the menu (Order 014 item 1c): a page hands the frame its files (`cx.drag_out(paths)` while the
//! button is held - an `Ev::Drag`), the frame starts Windows' own drag (the shell's data object + its default drop source)
//! after the page's event: Explorer, Discord, a mail draft take them like a drag from Explorer. Copy only.

use windows::core::HSTRING;
use windows::Win32::System::Com::IDataObject;
use windows::Win32::System::Ole::{IDropSource, OleInitialize, DROPEFFECT_COPY};
use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{ILCreateFromPathW, ILFree, SHCreateDataObject, SHDoDragDrop};

/// Run the drag (modal: it returns when the button comes up). True = the files were dropped somewhere.
pub fn drag_out(paths: &[String]) -> bool {
    unsafe {
        // drag and drop needs OLE on this thread (a second call only returns S_FALSE)
        let _ = OleInitialize(None);
        let pidls: Vec<*mut ITEMIDLIST> = paths.iter().map(|p| ILCreateFromPathW(&HSTRING::from(p.as_str()))).filter(|p| !p.is_null()).collect();
        if pidls.is_empty() {
            return false;
        }
        let refs: Vec<*const ITEMIDLIST> = pidls.iter().map(|p| *p as *const ITEMIDLIST).collect();
        // (no parent folder = the desktop: the item ids are absolute)
        let data: windows::core::Result<IDataObject> = SHCreateDataObject(None, Some(&refs), None::<&IDataObject>);
        // the menu holds the mouse since the button went down; the drag takes it over
        let _ = ReleaseCapture();
        let owner = crate::MENU_HWND.with(|h| *h.borrow());
        let r = data.and_then(|d| crate::modal(|| SHDoDragDrop(if owner.0.is_null() { None } else { Some(owner) }, &d, None::<&IDropSource>, DROPEFFECT_COPY)));
        for p in pidls {
            ILFree(Some(p as *const ITEMIDLIST));
        }
        r.is_ok_and(|e| e.0 != 0)
    }
}
