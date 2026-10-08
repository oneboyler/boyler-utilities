//! `shell:AppsFolder`: every app the Start menu would list (desktop programs and Store apps). Read-only.

use windows::core::{Interface, GUID, PWSTR};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::UI::Shell::{
    FOLDERID_AppsFolder, IEnumShellItems, IShellItem, IShellItem2, SHGetKnownFolderItem, BHID_EnumItems, KF_FLAG_DEFAULT,
    SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING,
};

use super::{os_err, Com};
use crate::error::Result;
use crate::os::AppEntry;

/// `System.Link.TargetParsingPath` — the program a desktop app's Start entry runs.
const PKEY_LINK_TARGET_PARSING_PATH: PROPERTYKEY =
    PROPERTYKEY { fmtid: GUID::from_u128(0xB9B4B3FC_2B51_4A42_B5D8_324146AFCF25), pid: 2 };

unsafe fn display_name(item: &IShellItem, kind: windows::Win32::UI::Shell::SIGDN) -> Option<String> {
    let p: PWSTR = item.GetDisplayName(kind).ok()?;
    let s = p.to_string().ok();
    CoTaskMemFree(Some(p.0 as *const _));
    s
}

pub fn list() -> Result<Vec<AppEntry>> {
    let _com = Com::sta();
    unsafe {
        let folder: IShellItem = SHGetKnownFolderItem(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None).map_err(|e| os_err("AppsFolder", e))?;
        let en: IEnumShellItems = folder.BindToHandler(None, &BHID_EnumItems).map_err(|e| os_err("AppsFolder enumerate", e))?;
        let mut out = Vec::new();
        loop {
            let mut one = [None];
            let mut got = 0u32;
            let hr = en.Next(&mut one, Some(&mut got));
            if hr.is_err() || got == 0 {
                break;
            }
            let Some(item) = one[0].take() else { break };
            let (Some(name), Some(parsing)) = (display_name(&item, SIGDN_NORMALDISPLAY), display_name(&item, SIGDN_PARENTRELATIVEPARSING)) else {
                continue;
            };
            let program_path = item.cast::<IShellItem2>().ok().and_then(|i2| {
                let p = i2.GetString(&PKEY_LINK_TARGET_PARSING_PATH).ok()?;
                let s = p.to_string().ok().filter(|s| !s.is_empty());
                CoTaskMemFree(Some(p.0 as *const _));
                s
            });
            out.push(AppEntry { name, parsing_name: parsing, program_path });
        }
        Ok(out)
    }
}
