//! Windows' own file / folder picker (Order 014 item 1c): the "click to pick one" of a page (Security's drop zone,
//! Screenshots' folder). Modal over the menu window; pages reach it only through `Cx::pick_folder` / `Cx::pick_file`
//! (only during a click, never in a test copy).

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{FileOpenDialog, IFileOpenDialog, IShellItem, SHCreateItemFromParsingName, FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, SIGDN_FILESYSPATH};

/// Show the picker: a folder (`folder`) or a file with `filters` ((name, "*.exe;*.lnk")...). None = cancelled / failed.
pub fn pick(title: &str, folder: bool, filters: &[(&str, &str)]) -> Option<String> {
    pick_in(title, folder, filters, None)
}

/// `pick`, opening in the folder `start` (Order 042: the cursor pickers open in Windows' own Cursors folder) - every
/// time, not only the first (IFileDialog::SetFolder); a folder that doesn't exist leaves Windows' choice.
pub fn pick_in(title: &str, folder: bool, filters: &[(&str, &str)], start: Option<&str>) -> Option<String> {
    unsafe {
        // (the UI thread is an STA already; a second call only returns S_FALSE)
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let d: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let mut o = d.GetOptions().ok()? | FOS_FORCEFILESYSTEM;
        if folder {
            o |= FOS_PICKFOLDERS;
        }
        d.SetOptions(o).ok()?;
        let t = HSTRING::from(title);
        let _ = d.SetTitle(PCWSTR(t.as_ptr()));
        if let Some(dir) = start {
            let w = HSTRING::from(dir);
            if let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(PCWSTR(w.as_ptr()), None) {
                let _ = d.SetFolder(&item);
            }
        }
        // the filter strings must live until Show returns
        let names: Vec<HSTRING> = filters.iter().map(|f| HSTRING::from(f.0)).collect();
        let specs: Vec<HSTRING> = filters.iter().map(|f| HSTRING::from(f.1)).collect();
        if !folder && !filters.is_empty() {
            let fs: Vec<COMDLG_FILTERSPEC> = names.iter().zip(&specs).map(|(n, s)| COMDLG_FILTERSPEC { pszName: PCWSTR(n.as_ptr()), pszSpec: PCWSTR(s.as_ptr()) }).collect();
            let _ = d.SetFileTypes(&fs);
        }
        let owner = crate::MENU_HWND.with(|h| *h.borrow());
        crate::modal(|| d.Show(if owner.0.is_null() { None } else { Some(owner) })).ok()?;
        let item = d.GetResult().ok()?;
        let p = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s
    }
}
