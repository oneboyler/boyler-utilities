//! Order 045: files dragged over the menu from Explorer - Windows' OLE drag and drop (`IDropTarget`), so a page sees the
//! drag HOVER, not only the drop (Security's drop zone lights up while a file is over it: menu-v22 `.sdz.over`, the
//! drawing's `dragenter` / `dragleave` / `drop`, L5779-5788). Registered on the menu window while it exists; replaces
//! `DragAcceptFiles` / WM_DROPFILES (which only told about the drop). Files only (CF_HDROP), copy.

use windows::core::{implement, Ref, Result};
use windows::Win32::Foundation::{HWND, POINT, POINTL};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Com::{IDataObject, DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
use windows::Win32::System::Ole::{IDropTarget, IDropTarget_Impl, OleInitialize, RegisterDragDrop, ReleaseStgMedium, RevokeDragDrop, CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

/// What the drag does over the window (client coordinates in physical pixels).
pub enum DropEv {
    /// files are over (x, y)
    Over(i32, i32),
    /// they left the window (or the drag was cancelled)
    Leave,
    /// they were dropped at (x, y)
    Drop(i32, i32, Vec<String>),
}

#[implement(IDropTarget)]
struct Target {
    hwnd: HWND,
    on: Box<dyn Fn(DropEv)>,
    files: std::cell::Cell<bool>,
}

fn fmt() -> FORMATETC {
    FORMATETC { cfFormat: CF_HDROP.0, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 }
}

/// The full paths in a data object that carries files.
fn paths(d: &IDataObject) -> Vec<String> {
    let mut out = Vec::new();
    unsafe {
        let Ok(mut m) = d.GetData(&fmt()) else { return out };
        let hd = HDROP(m.u.hGlobal.0);
        let n = DragQueryFileW(hd, u32::MAX, None);
        for i in 0..n {
            let len = DragQueryFileW(hd, i, None) as usize;
            let mut b = vec![0u16; len + 1];
            let got = DragQueryFileW(hd, i, Some(&mut b)) as usize;
            out.push(String::from_utf16_lossy(&b[..got]));
        }
        ReleaseStgMedium(&mut m);
    }
    out
}

impl Target {
    fn client(&self, pt: &POINTL) -> (i32, i32) {
        let mut p = POINT { x: pt.x, y: pt.y };
        unsafe {
            let _ = ScreenToClient(self.hwnd, &mut p);
        }
        (p.x, p.y)
    }
    fn effect(&self, e: *mut DROPEFFECT) {
        if !e.is_null() {
            unsafe { *e = if self.files.get() { DROPEFFECT_COPY } else { DROPEFFECT_NONE } };
        }
    }
}

impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(&self, data: Ref<IDataObject>, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let files = data.as_ref().is_some_and(|d| unsafe { d.QueryGetData(&fmt()) }.is_ok());
        self.files.set(files);
        self.effect(effect);
        if files {
            let (x, y) = self.client(pt);
            (self.on)(DropEv::Over(x, y));
        }
        Ok(())
    }
    fn DragOver(&self, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        self.effect(effect);
        if self.files.get() {
            let (x, y) = self.client(pt);
            (self.on)(DropEv::Over(x, y));
        }
        Ok(())
    }
    fn DragLeave(&self) -> Result<()> {
        if self.files.replace(false) {
            (self.on)(DropEv::Leave);
        }
        Ok(())
    }
    fn Drop(&self, data: Ref<IDataObject>, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        self.effect(effect);
        if self.files.replace(false) {
            let (x, y) = self.client(pt);
            let p = data.as_ref().map(paths).unwrap_or_default();
            (self.on)(DropEv::Leave);
            if !p.is_empty() {
                (self.on)(DropEv::Drop(x, y, p));
            }
        }
        Ok(())
    }
}

/// Take drags on the window `hwnd` (its thread must pump messages; OLE needs a single-threaded apartment there).
/// False = Windows refused (the caller falls back to `DragAcceptFiles`, drops only).
pub fn register(hwnd: HWND, on: impl Fn(DropEv) + 'static) -> bool {
    thread_local!(static OLE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) });
    unsafe {
        // once per thread (it stays for the app's life; drag-out shares it)
        if !OLE.with(|o| o.replace(true)) {
            let _ = OleInitialize(None);
        }
        let t: IDropTarget = Target { hwnd, on: Box::new(on), files: std::cell::Cell::new(false) }.into();
        RegisterDragDrop(hwnd, &t).is_ok()
    }
}

/// The window goes: stop taking drags (Windows holds the target until then).
pub fn revoke(hwnd: HWND) {
    unsafe {
        let _ = RevokeDragDrop(hwnd);
    }
}
