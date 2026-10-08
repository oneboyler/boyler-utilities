//! Settings › Glass style (dark theme only; light glass keeps its own look). The numbers are the owner's (QUEUE §B 11) as the
//! drawing writes them (menu-v22.html, the v19 / v21 glass block: `#sw:not(.light)`, `#sw.gl-fro`, `#sw.gl-win`).
//! The rim / sheen box-shadows of each style are painting detail and stay with the glass painter.

/// The three glass styles; Liquid is the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GlassStyle {
    #[default]
    Liquid,
    Frosted,
    WindowsLook,
}

/// One style's numbers, straight from the drawing's CSS.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlassNumbers {
    /// `--tint`: the window tint colour (r, g, b) and its alpha.
    pub tint_rgb: [u8; 3],
    pub tint_alpha: f32,
    /// `backdrop-filter: blur(Npx)`.
    pub blur_px: f32,
    /// `saturate(N %)` as a factor (170 % = 1.70).
    pub saturate: f32,
    /// `brightness(N)` (1.0 = none).
    pub brightness: f32,
    /// `--grp`: the option bubbles, white at this alpha.
    pub bubbles_alpha: f32,
    /// `--hl`: the highlight colour, white at this alpha.
    pub highlight_alpha: f32,
}

impl GlassStyle {
    pub const ALL: [GlassStyle; 3] = [GlassStyle::Liquid, GlassStyle::Frosted, GlassStyle::WindowsLook];

    /// The id written to the settings file.
    pub fn id(self) -> &'static str {
        match self {
            GlassStyle::Liquid => "liquid",
            GlassStyle::Frosted => "frosted",
            GlassStyle::WindowsLook => "windows",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.id() == id)
    }

    /// The name the Settings dropdown shows (drawing: "Liquid · Frosted · Windows look").
    pub fn label(self) -> &'static str {
        match self {
            GlassStyle::Liquid => "Liquid",
            GlassStyle::Frosted => "Frosted",
            GlassStyle::WindowsLook => "Windows look",
        }
    }

    pub fn numbers(self) -> GlassNumbers {
        match self {
            // #sw:not(.light){--tint:rgba(20,20,26,.22);--grp:rgba(255,255,255,.08);--hl:rgba(255,255,255,.22);
            //   backdrop-filter:blur(13px) saturate(170%) brightness(1.04)}
            GlassStyle::Liquid => GlassNumbers {
                tint_rgb: [20, 20, 26],
                tint_alpha: 0.22,
                blur_px: 13.0,
                saturate: 1.70,
                brightness: 1.04,
                bubbles_alpha: 0.08,
                highlight_alpha: 0.22,
            },
            // #sw.gl-fro:not(.light){--tint:rgba(28,28,32,.40);--grp:rgba(255,255,255,.07);--hl:rgba(255,255,255,.14);
            //   backdrop-filter:blur(24px) saturate(150%)}
            GlassStyle::Frosted => GlassNumbers {
                tint_rgb: [28, 28, 32],
                tint_alpha: 0.40,
                blur_px: 24.0,
                saturate: 1.50,
                brightness: 1.0,
                bubbles_alpha: 0.07,
                highlight_alpha: 0.14,
            },
            // #sw.gl-win:not(.light){--tint:rgba(44,44,46,.75);--grp:rgba(255,255,255,.06);--hl:rgba(255,255,255,.1);
            //   backdrop-filter:blur(31px) saturate(115%)}
            GlassStyle::WindowsLook => GlassNumbers {
                tint_rgb: [44, 44, 46],
                tint_alpha: 0.75,
                blur_px: 31.0,
                saturate: 1.15,
                brightness: 1.0,
                bubbles_alpha: 0.06,
                highlight_alpha: 0.10,
            },
        }
    }
}

impl GlassNumbers {
    /// The light glass (Order 033) - one look for every glass style, as the drawing has it (its style rules are all
    /// `:not(.light)`): `#sw.light{--tint:rgba(246,246,248,.68);--grp:rgba(255,255,255,.62);--hl:rgba(255,255,255,.75)}` over
    /// `#sw{backdrop-filter:blur(40px) saturate(180%)}`.
    pub const LIGHT: GlassNumbers = GlassNumbers {
        tint_rgb: [246, 246, 248],
        tint_alpha: 0.68,
        blur_px: 40.0,
        saturate: 1.80,
        brightness: 1.0,
        bubbles_alpha: 0.62,
        highlight_alpha: 0.75,
    };
}
