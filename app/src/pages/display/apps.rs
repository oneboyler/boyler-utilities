//! The apps a "Switch automatically" rule can pick from its app list (the drawing's `RAPPS`: name, tile glyph, tile
//! colours) with the exe each one runs as. Any other program comes from "Browse for an app…" (its .exe path).

use crate::gfx::Rgba;

pub struct App {
    pub name: &'static str,
    /// the process the rule watches (its file name; matched case-insensitively, bu_display::autoswitch::exe_matches)
    pub exe: &'static str,
    /// the drawing's ICON name on the tile
    pub glyph: &'static str,
    /// the tile's `linear-gradient(135deg, a, b)`
    pub grad: (Rgba, Rgba),
}

const fn hx(h: u32) -> Rgba {
    Rgba(((h >> 16) & 255) as f32 / 255.0, ((h >> 8) & 255) as f32 / 255.0, (h & 255) as f32 / 255.0, 1.0)
}

/// menu-v22 `RAPPS`, in its order. Exe names: the games' own shipping executables (Minecraft = the Windows / Bedrock
/// edition's; the Java edition runs as javaw.exe, which every Java program uses, so it is left to "Browse").
pub const KNOWN: [App; 6] = [
    App { name: "VALORANT", exe: "VALORANT-Win64-Shipping.exe", glyph: "pad", grad: (hx(0xff7a76), hx(0xd83f4c)) },
    App { name: "Counter-Strike 2", exe: "cs2.exe", glyph: "aim", grad: (hx(0xffc56b), hx(0xe0861c)) },
    App { name: "Fortnite", exe: "FortniteClient-Win64-Shipping.exe", glyph: "pad", grad: (hx(0xb58cff), hx(0x6f4ae0)) },
    App { name: "Apex Legends", exe: "r5apex.exe", glyph: "tri", grad: (hx(0xff8f6b), hx(0xc4422f)) },
    App { name: "Rocket League", exe: "RocketLeague.exe", glyph: "globe", grad: (hx(0x5ab4ff), hx(0x2a74e6)) },
    App { name: "Minecraft", exe: "Minecraft.Windows.exe", glyph: "cube", grad: (hx(0x7ed67a), hx(0x3f9a3b)) },
];

/// The known app a rule's exe is, if any.
pub fn known(exe: &str) -> Option<&'static App> {
    KNOWN.iter().find(|a| bu_display::autoswitch::exe_matches(a.exe, exe))
}

/// The name a rule row shows: the known app's name, else the exe's file name without ".exe".
pub fn name_of(exe: &str) -> String {
    if let Some(a) = known(exe) {
        return a.name.to_string();
    }
    let f = exe.rsplit(['\\', '/']).next().unwrap_or(exe);
    f.strip_suffix(".exe").or_else(|| f.strip_suffix(".EXE")).unwrap_or(f).to_string()
}
