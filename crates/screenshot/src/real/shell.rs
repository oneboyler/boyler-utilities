//! Clipboard, Recycle Bin, Explorer, the folder picker and the drag-out data object.

use std::collections::BTreeMap;
use std::mem::ManuallyDrop;
use std::path::{Path, PathBuf};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{GlobalFree, ERROR_CANCELLED, HANDLE, HGLOBAL, HWND};
use windows::Win32::System::Com::{
    CoCreateInstance, CoTaskMemFree, IBindCtx, IDataObject, CLSCTX_INPROC_SERVER, DVASPECT_CONTENT, FORMATETC, STGMEDIUM, STGMEDIUM_0,
    TYMED_HGLOBAL,
};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::{CF_DIB, CF_HDROP};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    BHID_DataObject, FileOpenDialog, IFileOpenDialog, IShellItem, ILCreateFromPathW, ILFree, SHCreateItemFromParsingName, SHCreateShellItemArrayFromIDLists,
    SHFileOperationW, SHOpenFolderAndSelectItems, SHQueryRecycleBinW, ShellExecuteW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION,
    FOF_NOERRORUI, FOF_SILENT, FOF_WANTNUKEWARNING, FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FO_DELETE, SHFILEOPSTRUCTW,
    SHQUERYRBINFO, SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, HWND_MESSAGE, SW_SHOWNORMAL, WINDOW_EX_STYLE, WINDOW_STYLE};

use super::{ComGuard, Ctx};
use crate::encode;
use crate::error::{Error, Result};

fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    s.encode_wide().chain(std::iter::once(0)).collect()
}

/// Movable global memory holding `bytes` (what the clipboard and OLE data objects take).
fn hglobal(bytes: &[u8]) -> Result<HGLOBAL> {
    let h = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)) }.ctx("GlobalAlloc")?;
    let p = unsafe { GlobalLock(h) };
    if p.is_null() {
        let _ = unsafe { GlobalFree(Some(h)) };
        return Err(Error::os("GlobalLock", 0));
    }
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p as *mut u8, bytes.len());
        let _ = GlobalUnlock(h); // "fails" with no error once the lock count reaches 0
    }
    Ok(h)
}

/// A hidden message-only window (never on screen) to own the clipboard while we fill it — with no owner window,
/// SetClipboardData is documented to fail after EmptyClipboard.
struct ClipboardOpen {
    hwnd: HWND,
}

impl ClipboardOpen {
    fn open() -> Result<Self> {
        let hwnd = unsafe {
            CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), w!(""), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, None, None)
        }
        .ctx("CreateWindowExW (message-only)")?;
        // Another app may hold the clipboard for a moment: try for up to ~0.5 s.
        let mut last = None;
        for _ in 0..25 {
            match unsafe { OpenClipboard(Some(hwnd)) } {
                Ok(()) => return Ok(ClipboardOpen { hwnd }),
                Err(e) => last = Some(e),
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let _ = unsafe { DestroyWindow(hwnd) };
        Err(Error::os("OpenClipboard", last.map(|e| e.code().0).unwrap_or(0)))
    }

    fn put(&self, format: u32, bytes: &[u8]) -> Result<()> {
        let h = hglobal(bytes)?;
        if let Err(e) = unsafe { SetClipboardData(format, Some(HANDLE(h.0))) } {
            let _ = unsafe { GlobalFree(Some(h)) }; // still ours when the call failed
            return Err(Error::os("SetClipboardData", e.code().0));
        }
        Ok(()) // the clipboard owns the memory now
    }
}

impl Drop for ClipboardOpen {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

/// The picture: registered format "PNG" (Discord, browsers, Office, GIMP read it — keeps exact pixels) and CF_DIB (everything
/// else; Windows makes CF_BITMAP / CF_DIBV5 from it on request).
pub fn set_clipboard_picture(png: &[u8], dib: &[u8]) -> Result<()> {
    let png_fmt = unsafe { RegisterClipboardFormatW(w!("PNG")) };
    let c = ClipboardOpen::open()?;
    unsafe { EmptyClipboard() }.ctx("EmptyClipboard")?;
    c.put(png_fmt, png)?;
    c.put(CF_DIB.0 as u32, dib)
}

/// Files, the way Explorer's Copy puts them: CF_HDROP + "Preferred DropEffect" = DROPEFFECT_COPY (1).
pub fn set_clipboard_files(paths: &[PathBuf]) -> Result<()> {
    let effect_fmt = unsafe { RegisterClipboardFormatW(w!("Preferred DropEffect")) };
    let c = ClipboardOpen::open()?;
    unsafe { EmptyClipboard() }.ctx("EmptyClipboard")?;
    c.put(CF_HDROP.0 as u32, &encode::dropfiles_bytes(paths))?;
    c.put(effect_fmt, &1u32.to_le_bytes())
}

/// The drive root of a path (`C:\`, `\\server\share\`).
fn root_of(p: &Path) -> PathBuf {
    p.ancestors().last().map(Path::to_path_buf).unwrap_or_else(|| p.to_path_buf())
}

/// To the Recycle Bin, no confirm, no progress window (DESIGN: "to the Recycle Bin, with no confirm").
/// - A drive with no Recycle Bin (SHQueryRecycleBin fails) → [`Error::NoRecycleBin`], nothing deleted.
/// - FOF_WANTNUKEWARNING: if Windows would delete a file for good anyway (e.g. too big for the bin), it asks first instead of
///   silently destroying it.
pub fn recycle(paths: &[PathBuf]) -> Result<()> {
    for p in paths {
        let root = wide(root_of(p).as_os_str());
        let mut info = SHQUERYRBINFO { cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32, ..Default::default() };
        if unsafe { SHQueryRecycleBinW(PCWSTR(root.as_ptr()), &mut info) }.is_err() {
            return Err(Error::NoRecycleBin(p.clone()));
        }
    }
    // pFrom: full paths, each NUL-terminated, the list ended by an extra NUL.
    let mut from: Vec<u16> = Vec::new();
    for p in paths {
        from.extend(wide(p.as_os_str()));
    }
    from.push(0);
    let mut op = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: PCWSTR(from.as_ptr()),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI | FOF_WANTNUKEWARNING).0 as u16,
        ..Default::default()
    };
    let rc = unsafe { SHFileOperationW(&mut op) };
    if rc != 0 {
        return Err(Error::os("SHFileOperationW (delete)", rc));
    }
    if op.fAnyOperationsAborted.as_bool() {
        return Err(Error::os("SHFileOperationW (aborted)", 0));
    }
    Ok(())
}

/// Absolute shell item ids for paths; freed on drop.
struct Pidls(Vec<*mut ITEMIDLIST>);

impl Pidls {
    fn of(paths: &[&Path]) -> Result<Self> {
        let mut v = Pidls(Vec::with_capacity(paths.len()));
        for p in paths {
            let w = wide(p.as_os_str());
            let id = unsafe { ILCreateFromPathW(PCWSTR(w.as_ptr())) };
            if id.is_null() {
                return Err(Error::Io { what: p.display().to_string(), why: "not found by the shell".into() });
            }
            v.0.push(id);
        }
        Ok(v)
    }

    fn as_const(&self) -> Vec<*const ITEMIDLIST> {
        self.0.iter().map(|p| *p as *const _).collect()
    }
}

impl Drop for Pidls {
    fn drop(&mut self) {
        for p in &self.0 {
            unsafe { ILFree(Some(*p as *const _)) };
        }
    }
}

/// Explorer with the files selected — one window per folder (SHOpenFolderAndSelectItems).
pub fn show_in_folder(paths: &[PathBuf]) -> Result<()> {
    let _com = ComGuard::sta()?;
    let mut by_dir: BTreeMap<PathBuf, Vec<&Path>> = BTreeMap::new();
    for p in paths {
        if let Some(d) = p.parent() {
            by_dir.entry(d.to_path_buf()).or_default().push(p);
        }
    }
    for (dir, files) in by_dir {
        let folder = Pidls::of(&[dir.as_path()])?;
        let items = Pidls::of(&files)?;
        unsafe { SHOpenFolderAndSelectItems(folder.0[0], Some(&items.as_const()), 0) }.ctx("SHOpenFolderAndSelectItems")?;
    }
    Ok(())
}

/// The folder in Explorer.
pub fn open_folder(dir: &Path) -> Result<()> {
    let w = wide(dir.as_os_str());
    let h = unsafe { ShellExecuteW(None, w!("open"), PCWSTR(w.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
    if (h.0 as isize) <= 32 {
        return Err(Error::os("ShellExecuteW (open folder)", h.0 as isize as i64));
    }
    Ok(())
}

/// Windows' folder picker (IFileOpenDialog with FOS_PICKFOLDERS), starting in `start`.
pub fn pick_folder(start: Option<&Path>) -> Result<PathBuf> {
    let _com = ComGuard::sta()?;
    let dlg: IFileOpenDialog = unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) }.ctx("FileOpenDialog")?;
    unsafe {
        let opts = dlg.GetOptions().ctx("GetOptions")?;
        dlg.SetOptions(opts | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM).ctx("SetOptions")?;
        if let Some(s) = start {
            let w = wide(s.as_os_str());
            if let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(PCWSTR(w.as_ptr()), None) {
                let _ = dlg.SetFolder(&item);
            }
        }
        if let Err(e) = dlg.Show(None) {
            if e.code() == ERROR_CANCELLED.to_hresult() {
                return Err(Error::Cancelled);
            }
            return Err(Error::os("IFileOpenDialog::Show", e.code().0));
        }
        let item = dlg.GetResult().ctx("GetResult")?;
        let p = item.GetDisplayName(SIGDN_FILESYSPATH).ctx("GetDisplayName")?;
        let s = p.to_string();
        CoTaskMemFree(Some(p.0 as *const _));
        Ok(PathBuf::from(s.map_err(|_| Error::BadData("picked path".into()))?))
    }
}

/// The shell data object for dragging shots out (DESIGN "Drag out": drop into Discord, Explorer, anywhere): the data object
/// of a shell item array made from the files' absolute shell ids (BHID_DataObject) — it carries CF_HDROP (what Discord, browsers and most apps read) and
/// the shell's own formats (what Explorer reads) — plus "Preferred DropEffect" = copy, so Explorer copies instead of moving
/// the file out of the screenshots folder. Files may sit in different folders.
///
/// The calling thread must have OLE initialised (the menu's UI thread does, for DoDragDrop). The UI calls
/// `DoDragDrop(&obj, source, DROPEFFECT_COPY, …)`.
pub fn drag_data_object(paths: &[PathBuf]) -> Result<IDataObject> {
    let refs: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();
    let pidls = Pidls::of(&refs)?;
    let items = unsafe { SHCreateShellItemArrayFromIDLists(&pidls.as_const()) }.ctx("SHCreateShellItemArrayFromIDLists")?;
    let obj: IDataObject = unsafe { items.BindToHandler(None::<&IBindCtx>, &BHID_DataObject) }.ctx("BindToHandler (data object)")?;
    let effect_fmt = unsafe { RegisterClipboardFormatW(w!("Preferred DropEffect")) };
    let fe = FORMATETC {
        cfFormat: effect_fmt as u16,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    let h = hglobal(&1u32.to_le_bytes())?;
    let sm = STGMEDIUM { tymed: TYMED_HGLOBAL.0 as u32, u: STGMEDIUM_0 { hGlobal: h }, pUnkForRelease: ManuallyDrop::new(None) };
    // fRelease = true: the data object owns the memory from here on.
    if let Err(e) = unsafe { obj.SetData(&fe, &sm, true) } {
        let _ = unsafe { GlobalFree(Some(h)) };
        return Err(Error::os("IDataObject::SetData (Preferred DropEffect)", e.code().0));
    }
    Ok(obj)
}
