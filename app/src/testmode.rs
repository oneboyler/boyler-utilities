//! Test mode (Order 014, A_INT_01): a copy started with any test switch is a TEST copy and lives apart from the real one.
//! - its own single-instance mutex, message-window class and menu/shadow window classes, so a test start never hands
//!   anything to a normal copy (or the other way round) and a test script that looks for the menu finds only the test one;
//! - its own WM_COPYDATA magic: a normal copy drops every message that carries the test one;
//! - a normal copy refuses every test-only command (it accepts only the tray's own entry points);
//! - the test switches read from the environment (BU_*) count only in a test copy.
//!   A test copy never shows a tray icon and uses the FAKE services unless started with `--real-read` (then it only reads,
//!   and every command or input that would change something is refused).

use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::{w, PCWSTR};

static TEST: AtomicBool = AtomicBool::new(false);
static REAL_READ: AtomicBool = AtomicBool::new(false);
static DEMO: AtomicBool = AtomicBool::new(false);

pub fn set(test: bool, real_read: bool) {
    TEST.store(test, Ordering::Relaxed);
    REAL_READ.store(test && real_read, Ordering::Relaxed);
}

/// This copy was started in test mode.
pub fn on() -> bool {
    TEST.load(Ordering::Relaxed)
}

/// A test copy that reads the real services (measuring): nothing may change anything.
pub fn real_read() -> bool {
    REAL_READ.load(Ordering::Relaxed)
}

/// `--demo-names` (Order 044, the README pictures): a test copy shows obviously made-up names instead of the sample data's
/// real app, game and device names. Only in a test copy.
pub fn set_demo(on: bool) {
    DEMO.store(on && self::on(), Ordering::Relaxed);
}

/// Real name -> made-up name, longest first (substrings of a text are replaced).
const DEMO_NAMES: [(&str, &str); 37] = [
    ("DualSense Edge Wireless Controller", "Nova Pad Pro"),
    ("DualSense Edge", "Nova Pad Pro"),
    ("DualSense", "Nova Pad"),
    ("Pulsar X2 CrazyLight", "Comet M2 Lite"),
    ("Pulsar", "Comet"),
    ("Headphones (Arctis Nova)", "Headphones (Nebula X7)"),
    ("Microphone (Shure MV7)", "Microphone (Echo One)"),
    ("Arctis Nova", "Nebula X7"),
    ("Shure MV7", "Echo One"),
    ("Rocket League", "Turbo Kart League"),
    ("Counter-Strike 2", "Sector Strike"),
    ("Call of Duty", "Frontline Ops"),
    ("Apex Legends", "Rift Legends"),
    ("Fortnite", "Skybuild"),
    ("VALORANT", "STARFALL"),
    ("Valorant", "Starfall"),
    ("CS2", "Sector Strike"),
    ("Spotifast", "Tunewave"),
    ("Spotify", "Tunewave"),
    ("Discord", "Chatterbox"),
    ("Google Chrome", "Orbit Browser"),
    ("Chrome", "Orbit"),
    ("Steam Deck", "Pocket Deck"),
    ("Riot Games", "Starfall Studio"),
    ("Riot Vanguard", "Starfall Guard"),
    ("Epic Games", "Skybuild Games"),
    ("WireGuard", "Tunnel"),
    ("Cloudflare", "Speedline"),
    ("VirtualBox", "LabBox"),
    ("AMD Ryzen 7 7800X3D", "Zentrix 8-core"),
    ("Ryzen 7 7800X3D", "Zentrix 8-core"),
    ("NVIDIA GeForce RTX 4070 SUPER", "Prism GX 70"),
    ("RTX 4070 SUPER", "Prism GX 70"),
    ("ASUS ROG STRIX B650E-F GAMING WIFI", "Meridian B6 Board"),
    ("Samsung 990 PRO", "Swift NVMe 2TB"),
    ("Wootility", "KeyForge"),
    ("Adobe Premiere Pro 2026", "ClipStudio 2026"),
];

/// A text as the demo shows it (unchanged unless `--demo-names`).
pub fn demo_text(s: String) -> String {
    if !DEMO.load(Ordering::Relaxed) {
        return s;
    }
    let mut s = s;
    for (real, fake) in DEMO_NAMES {
        if s.contains(real) {
            s = s.replace(real, fake);
        }
    }
    s
}

/// An environment test switch: honoured only in a test copy.
pub fn env(name: &str) -> Option<String> {
    if on() {
        std::env::var(name).ok()
    } else {
        None
    }
}

pub const MAGIC_NORMAL: usize = 0x4255_4E31; // "BUN1"
pub const MAGIC_TEST: usize = 0x4255_5431; // "BUT1"

pub fn magic() -> usize {
    if on() {
        MAGIC_TEST
    } else {
        MAGIC_NORMAL
    }
}

pub fn mutex_name() -> PCWSTR {
    if on() {
        w!("Local\\BoylerUtilities.Test")
    } else {
        w!("Local\\BoylerUtilities")
    }
}

pub fn msg_class() -> PCWSTR {
    if on() {
        w!("BoylerUtilities.Test.Msg")
    } else {
        w!("BoylerUtilities.Msg")
    }
}

pub fn menu_class() -> PCWSTR {
    if on() {
        w!("BoylerUtilities.Test.Menu")
    } else {
        w!("BoylerUtilities.Menu")
    }
}

pub fn shadow_class() -> PCWSTR {
    if on() {
        w!("BoylerUtilities.Test.Shadow")
    } else {
        w!("BoylerUtilities.Shadow")
    }
}

/// The commands every copy accepts: the tray's own entry points (open / close / toggle / tab / quit).
const ENTRY: [&str; 5] = ["open", "close", "toggle", "tab", "quit"];
/// Test commands that change nothing (they look, or warm the graphics device up like a tray hover): allowed in a
/// `--real-read` test copy (measure.ps1).
const LOOK: [&str; 5] = ["snap", "snapover", "where", "state", "trayhover"];

/// Does this copy (test or normal, real-read or not) accept a command `cmd` that arrived with WM_COPYDATA magic `magic`
/// (None = from its own command line)?
pub fn accepts(test: bool, real_read: bool, magic: Option<usize>, cmd: &str) -> bool {
    let own = if test { MAGIC_TEST } else { MAGIC_NORMAL };
    if magic.is_some_and(|m| m != own) {
        return false;
    }
    let key = cmd.split_once(':').map(|(k, _)| k).unwrap_or(cmd);
    if ENTRY.contains(&key) {
        return true;
    }
    if !test {
        return false;
    }
    !real_read || LOOK.contains(&key)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_ONLY: [&str; 19] = [
        "key:70",
        "showmenu:vtt|listen",
        "scroll:400",
        "rclick:el:apps.row/0",
        "drop:el:sec.dz|C:\\x.exe",
        "click:outpick",
        "hover:dock:1",
        "type:250",
        "enter",
        "esc",
        "trayhover",
        "snap:C:\\x.png",
        "snapover:a|b",
        "switchshots:aud|C:\\d",
        "where:dock:1|C:\\w.txt",
        "state:C:\\s.txt",
        "snapscreen:a|b",
        "winlight:1",
        "anything-new",
    ];

    #[test]
    fn a_normal_copy_ignores_every_test_command() {
        for c in TEST_ONLY {
            assert!(!accepts(false, false, Some(MAGIC_NORMAL), c), "normal copy accepted {c}");
            assert!(!accepts(false, false, None, c), "normal copy accepted {c} from its command line");
        }
    }

    #[test]
    fn a_normal_copy_drops_messages_from_a_test_start() {
        for c in ["open", "close", "toggle", "tab:aud", "quit", "click:outpick", "state:x"] {
            assert!(!accepts(false, false, Some(MAGIC_TEST), c), "normal copy took {c} from a test start");
        }
    }

    #[test]
    fn a_test_copy_drops_messages_from_a_normal_start() {
        for c in ["open", "quit", "click:outpick"] {
            assert!(!accepts(true, false, Some(MAGIC_NORMAL), c), "test copy took {c} from a normal start");
        }
    }

    #[test]
    fn the_entry_points_reach_both() {
        for c in ["open", "close", "toggle", "tab:aud", "quit"] {
            assert!(accepts(false, false, Some(MAGIC_NORMAL), c));
            assert!(accepts(true, false, Some(MAGIC_TEST), c));
        }
    }

    #[test]
    fn a_test_copy_takes_test_commands() {
        for c in TEST_ONLY {
            assert!(accepts(true, false, Some(MAGIC_TEST), c), "test copy refused {c}");
        }
    }

    #[test]
    fn a_real_read_test_copy_only_looks() {
        for c in ["click:outpick", "hover:x", "type:5", "enter", "esc", "switchshots:a|b"] {
            assert!(!accepts(true, true, Some(MAGIC_TEST), c), "real-read copy accepted {c}");
        }
        for c in ["state:x", "snap:x", "where:a|b", "trayhover", "open", "tab:aud", "quit"] {
            assert!(accepts(true, true, Some(MAGIC_TEST), c), "real-read copy refused {c}");
        }
    }

    #[test]
    fn names_differ_between_modes() {
        set(false, false);
        let n = (mutex_name(), msg_class(), menu_class(), shadow_class(), magic());
        set(true, false);
        let t = (mutex_name(), msg_class(), menu_class(), shadow_class(), magic());
        set(false, false);
        unsafe {
            assert_ne!(n.0.to_string().unwrap(), t.0.to_string().unwrap());
            assert_ne!(n.1.to_string().unwrap(), t.1.to_string().unwrap());
            assert_ne!(n.2.to_string().unwrap(), t.2.to_string().unwrap());
            assert_ne!(n.3.to_string().unwrap(), t.3.to_string().unwrap());
        }
        assert_ne!(n.4, t.4);
    }
}
