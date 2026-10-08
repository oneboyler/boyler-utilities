//! A fake Steam built from the hand-written Steam-format fixtures (tests/fixtures; Steam's own layout: tab indents,
//! `"key"\t\t"value"`, LF — Steam's real files are never copied into the repo, A_015_01).
#![allow(dead_code)]

use bu_controller::fake::FakeSteam;
use bu_controller::steam::SteamPaths;
use bu_controller::ControllerService;
use std::path::PathBuf;

pub const RL: &str = include_str!("../fixtures/rl_ps5.vdf");
pub const RL_OFFICIAL: &str = include_str!("../fixtures/rl_official_legacy.bin");
pub const COMMUNITY: &str = include_str!("../fixtures/community_legacy.bin");
pub const CONFIGSET: &str = include_str!("../fixtures/configset_ps5.vdf");
pub const PREFS: &str = include_str!("../fixtures/preferences_edge.vdf");
pub const SERIAL: &str = "DSE000000000001";

pub const STEAM: &str = r"C:\Steam";
pub const ACCOUNT: &str = "10000001";

pub fn config() -> PathBuf {
    PathBuf::from(STEAM).join(r"steamapps\common\Steam Controller Configs").join(ACCOUNT).join("config")
}
pub fn rl_path() -> PathBuf {
    config().join(r"252950\controller_ps5.vdf")
}
pub fn yakuza_own_path() -> PathBuf {
    config().join(r"638970\controller_ps5.vdf")
}
pub fn configset_path() -> PathBuf {
    config().join("configset_controller_ps5.vdf")
}
pub fn prefs_path() -> PathBuf {
    config().join(format!("preferences_{SERIAL}.vdf"))
}
pub fn official_path() -> PathBuf {
    PathBuf::from(STEAM).join(r"steamapps\workshop\content\241100\1700935741\932716421548274678_legacy.bin")
}
pub fn community_path() -> PathBuf {
    PathBuf::from(STEAM).join(r"steamapps\workshop\content\241100\3275392801\2477623897035151804_legacy.bin")
}
pub const BACKUPS: &str = r"C:\BU\backups";

/// The binary shortcuts.vdf Steam writes (measured shape), with one shortcut.
pub fn shortcuts_bin(name: &str) -> Vec<u8> {
    let mut b = vec![0u8];
    b.extend(b"shortcuts\0");
    b.push(0);
    b.extend(b"0\0");
    b.push(2);
    b.extend(b"appid\0");
    b.extend([0x12, 0x34, 0x56, 0x78]);
    b.push(1);
    b.extend(b"AppName\0");
    b.extend(name.as_bytes());
    b.push(0);
    b.extend([8, 8, 8]);
    b
}

/// Every file of the fake Steam tree.
pub fn files() -> Vec<(PathBuf, Vec<u8>)> {
    let s = PathBuf::from(STEAM);
    vec![
        (s.join(r"steam.exe"), b"MZ".to_vec()),
        (configset_path(), CONFIGSET.into()),
        (rl_path(), RL.into()),
        (config().join(r"epic games launcher\controller_ps5.vdf"), RL_OFFICIAL.into()),
        (prefs_path(), PREFS.into()),
        (official_path(), RL_OFFICIAL.into()),
        (community_path(), COMMUNITY.into()),
        (s.join(r"steamapps\libraryfolders.vdf"), include_bytes!("../fixtures/libraryfolders.vdf").to_vec()),
        (s.join(r"steamapps\appmanifest_252950.acf"), include_bytes!("../fixtures/appmanifest_252950.vdf").to_vec()),
        (PathBuf::from(r"D:\SteamLibrary\steamapps\appmanifest_638970.acf"), include_bytes!("../fixtures/appmanifest_638970.vdf").to_vec()),
        (s.join(r"userdata").join(ACCOUNT).join(r"config\localconfig.vdf"), include_bytes!("../fixtures/localconfig.vdf").to_vec()),
        (s.join(r"userdata").join(ACCOUNT).join(r"config\shortcuts.vdf"), shortcuts_bin("Epic Games Launcher")),
    ]
}

pub fn fake() -> FakeSteam {
    let f = FakeSteam::new(STEAM);
    for (p, b) in files() {
        f.put(p, b);
    }
    f
}

pub fn service() -> ControllerService<FakeSteam> {
    let mut f = fake();
    f.active = Some(ACCOUNT.parse().unwrap());
    ControllerService::new(f, BACKUPS).expect("fake Steam found")
}

pub fn paths() -> SteamPaths {
    SteamPaths { dir: STEAM.into(), account: ACCOUNT.into() }
}

/// Lines removed from `old` and added in `new` (a plain LCS line diff) — the byte-identical-rest proof.
pub fn line_diff(old: &str, new: &str) -> (Vec<String>, Vec<String>) {
    let a: Vec<&str> = old.split_inclusive('\n').collect();
    let b: Vec<&str> = new.split_inclusive('\n').collect();
    let (n, m) = (a.len(), b.len());
    let mut l = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            l[i][j] = if a[i] == b[j] { l[i + 1][j + 1] + 1 } else { l[i + 1][j].max(l[i][j + 1]) };
        }
    }
    let (mut i, mut j, mut rem, mut add) = (0, 0, vec![], vec![]);
    while i < n && j < m {
        if a[i] == b[j] {
            i += 1;
            j += 1;
        } else if l[i + 1][j] >= l[i][j + 1] {
            rem.push(a[i].to_string());
            i += 1;
        } else {
            add.push(b[j].to_string());
            j += 1;
        }
    }
    rem.extend(a[i..].iter().map(|s| s.to_string()));
    add.extend(b[j..].iter().map(|s| s.to_string()));
    (rem, add)
}
