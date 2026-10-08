//! What a button "does": one Steam Input binding string ⇄ a plain [`Action`].
//!
//! A binding line in a layout looks like `"binding"  "key_press F5, , "`: the command, then `, <label>, <icon>` (Steam's own
//! saves always write the two empty fields; its templates leave them out). Commands seen in the 128 local layouts
//! (measured 2026-10-08): `xinput_button <B>`, `key_press <KEY>`, `mouse_button <B>`, `mouse_wheel SCROLL_UP|SCROLL_DOWN`,
//! `controller_action <WHAT> …`, `game_action <set> <action>` (games with Steam Input API actions — shown, never invented).

/// A gamepad button the virtual pad presses (Steam: `xinput_button`). Names follow the PlayStation pad; Xbox names in
/// [`PadButton::xbox_name`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PadButton {
    Cross,
    Circle,
    Square,
    Triangle,
    L1,
    R1,
    L2,
    R2,
    L3,
    R3,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
    /// PS: Create · Xbox: View (Steam: `select`)
    Create,
    /// PS: Options · Xbox: Menu (Steam: `start`)
    Options,
}

impl PadButton {
    pub const ALL: [PadButton; 16] = [
        PadButton::Cross,
        PadButton::Circle,
        PadButton::Square,
        PadButton::Triangle,
        PadButton::L1,
        PadButton::R1,
        PadButton::L2,
        PadButton::R2,
        PadButton::L3,
        PadButton::R3,
        PadButton::DpadUp,
        PadButton::DpadDown,
        PadButton::DpadLeft,
        PadButton::DpadRight,
        PadButton::Create,
        PadButton::Options,
    ];

    /// Steam's token, written the way Steam's own saves write it (its saves mix cases; both are read).
    pub fn steam(self) -> &'static str {
        match self {
            PadButton::Cross => "A",
            PadButton::Circle => "B",
            PadButton::Square => "X",
            PadButton::Triangle => "Y",
            PadButton::L1 => "shoulder_left",
            PadButton::R1 => "shoulder_right",
            PadButton::L2 => "TRIGGER_LEFT",
            PadButton::R2 => "TRIGGER_RIGHT",
            PadButton::L3 => "JOYSTICK_LEFT",
            PadButton::R3 => "JOYSTICK_RIGHT",
            PadButton::DpadUp => "dpad_up",
            PadButton::DpadDown => "dpad_down",
            PadButton::DpadLeft => "dpad_left",
            PadButton::DpadRight => "dpad_right",
            PadButton::Create => "select",
            PadButton::Options => "start",
        }
    }

    pub fn from_steam(s: &str) -> Option<PadButton> {
        let l = s.to_ascii_lowercase();
        PadButton::ALL.into_iter().find(|b| b.steam().eq_ignore_ascii_case(&l))
    }

    pub fn ps_name(self) -> &'static str {
        match self {
            PadButton::Cross => "Cross",
            PadButton::Circle => "Circle",
            PadButton::Square => "Square",
            PadButton::Triangle => "Triangle",
            PadButton::L1 => "L1",
            PadButton::R1 => "R1",
            PadButton::L2 => "L2",
            PadButton::R2 => "R2",
            PadButton::L3 => "L3",
            PadButton::R3 => "R3",
            PadButton::DpadUp => "D-pad up",
            PadButton::DpadDown => "D-pad down",
            PadButton::DpadLeft => "D-pad left",
            PadButton::DpadRight => "D-pad right",
            PadButton::Create => "Create",
            PadButton::Options => "Options",
        }
    }

    pub fn xbox_name(self) -> &'static str {
        match self {
            PadButton::Cross => "A",
            PadButton::Circle => "B",
            PadButton::Square => "X",
            PadButton::Triangle => "Y",
            PadButton::L1 => "LB",
            PadButton::R1 => "RB",
            PadButton::L2 => "LT",
            PadButton::R2 => "RT",
            PadButton::L3 => "LS",
            PadButton::R3 => "RS",
            PadButton::Create => "View",
            PadButton::Options => "Menu",
            other => other.ps_name(),
        }
    }
}

/// A mouse button (Steam: `mouse_button`). `Back` / `Forward` (mouse 4 / 5) are Steam's tokens found in steamclient64.dll;
/// no local layout uses them yet (guessed which token Steam's own picker writes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

impl MouseButton {
    pub const ALL: [MouseButton; 5] = [MouseButton::Left, MouseButton::Right, MouseButton::Middle, MouseButton::Back, MouseButton::Forward];
    pub fn steam(self) -> &'static str {
        match self {
            MouseButton::Left => "LEFT",
            MouseButton::Right => "RIGHT",
            MouseButton::Middle => "MIDDLE",
            MouseButton::Back => "BACK",
            MouseButton::Forward => "FORWARD",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            MouseButton::Left => "Left click",
            MouseButton::Right => "Right click",
            MouseButton::Middle => "Middle click",
            MouseButton::Back => "Mouse 4",
            MouseButton::Forward => "Mouse 5",
        }
    }
    fn from_steam(s: &str) -> Option<Self> {
        match s.to_ascii_uppercase().as_str() {
            "X1" => Some(MouseButton::Back),
            "X2" => Some(MouseButton::Forward),
            u => MouseButton::ALL.into_iter().find(|b| b.steam() == u),
        }
    }
}

/// Steam's own commands the drawing offers (Steam: `controller_action`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SteamAction {
    Screenshot,
    ShowKeyboard,
    /// `set_led R G B brightness? ? ?` — the drawing's "Light bar red" is `set_led 255 0 0 0 0 1` (a real Rocket League file).
    SetLed { r: u8, g: u8, b: u8, rest: String },
    /// Switch to another action set: `CHANGE_PRESET <n> …` (the numbers kept as they are).
    ChangePreset(String),
}

/// What one press does.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Action {
    /// No binding at all.
    Nothing,
    Pad(PadButton),
    /// A keyboard key by Steam's key name (`F5`, `SPACE`, `LEFT_SHIFT` …; see [`KEYS`]).
    Key(String),
    Mouse(MouseButton),
    WheelUp,
    WheelDown,
    Steam(SteamAction),
    /// Anything else (game actions, mode shifts, radial menus …): kept verbatim, never rewritten.
    Other(String),
}

/// The keyboard keys the drawing's picker offers: (shown name, Steam's token). Tokens measured in local layouts + the dll.
pub const KEYS: &[(&str, &str)] = &[
    ("Space", "SPACE"),
    ("Enter", "RETURN"),
    ("Esc", "ESCAPE"),
    ("Tab", "TAB"),
    ("Shift", "LEFT_SHIFT"),
    ("Ctrl", "LEFT_CONTROL"),
    ("Alt", "LEFT_ALT"),
    ("Backspace", "BACKSPACE"),
    ("F1", "F1"),
    ("F2", "F2"),
    ("F3", "F3"),
    ("F4", "F4"),
    ("F5", "F5"),
    ("F6", "F6"),
    ("F7", "F7"),
    ("F8", "F8"),
    ("F9", "F9"),
    ("F10", "F10"),
    ("F11", "F11"),
    ("F12", "F12"),
    ("Up", "UP_ARROW"),
    ("Down", "DOWN_ARROW"),
    ("Left", "LEFT_ARROW"),
    ("Right", "RIGHT_ARROW"),
    ("Home", "HOME"),
    ("End", "END"),
    ("Page Up", "PAGE_UP"),
    ("Page Down", "PAGE_DOWN"),
    ("Insert", "INSERT"),
    ("Del", "DELETE"),
];

/// The shown name of a Steam key token (`LEFT_SHIFT` → `Shift`, `A` → `A`).
pub fn key_label(token: &str) -> String {
    if let Some((l, _)) = KEYS.iter().find(|(_, t)| t.eq_ignore_ascii_case(token)) {
        return l.to_string();
    }
    match token.to_ascii_uppercase().as_str() {
        "ENTER" => "Enter".into(),
        "RIGHT_SHIFT" => "Right Shift".into(),
        "RIGHT_CONTROL" => "Right Ctrl".into(),
        "RIGHT_ALT" => "Right Alt".into(),
        u => u.replace('_', " "),
    }
}

/// The Steam token for a shown key name (`Shift` → `LEFT_SHIFT`, `q` → `Q`, `7` → `7`).
pub fn key_token(label: &str) -> Option<String> {
    if let Some((_, t)) = KEYS.iter().find(|(l, _)| l.eq_ignore_ascii_case(label)) {
        return Some(t.to_string());
    }
    let mut ch = label.chars();
    match (ch.next(), ch.next()) {
        (Some(c), None) if c.is_ascii_alphanumeric() => Some(c.to_ascii_uppercase().to_string()),
        _ => None,
    }
}

impl Action {
    /// Read one `binding` value. Never fails: anything unknown becomes [`Action::Other`].
    pub fn parse(binding: &str) -> Action {
        let cmd = binding.split(',').next().unwrap_or("").trim();
        if cmd.is_empty() {
            return Action::Nothing;
        }
        let mut parts = cmd.split_whitespace();
        let kind = parts.next().unwrap_or("").to_ascii_lowercase();
        let arg = parts.next().unwrap_or("");
        let rest: Vec<&str> = parts.collect();
        let other = || Action::Other(cmd.to_string());
        match kind.as_str() {
            "xinput_button" => PadButton::from_steam(arg).map(Action::Pad).unwrap_or_else(other),
            "key_press" if !arg.is_empty() && rest.is_empty() => Action::Key(arg.to_ascii_uppercase()),
            "mouse_button" => MouseButton::from_steam(arg).map(Action::Mouse).unwrap_or_else(other),
            "mouse_wheel" => match arg.to_ascii_uppercase().as_str() {
                "SCROLL_UP" => Action::WheelUp,
                "SCROLL_DOWN" => Action::WheelDown,
                _ => other(),
            },
            "controller_action" => match arg.to_ascii_uppercase().as_str() {
                "SCREENSHOT" => Action::Steam(SteamAction::Screenshot),
                "SHOW_KEYBOARD" => Action::Steam(SteamAction::ShowKeyboard),
                "EMPTY_BINDING" => Action::Nothing,
                "SET_LED" if rest.len() >= 3 => {
                    let n = |i: usize| rest.get(i).and_then(|s| s.parse::<u8>().ok());
                    match (n(0), n(1), n(2)) {
                        (Some(r), Some(g), Some(b)) => Action::Steam(SteamAction::SetLed { r, g, b, rest: rest[3..].join(" ") }),
                        _ => other(),
                    }
                }
                "CHANGE_PRESET" => Action::Steam(SteamAction::ChangePreset(rest.join(" "))),
                _ => other(),
            },
            _ => other(),
        }
    }

    /// The command text Steam reads (without the label / icon fields).
    pub fn command(&self) -> Option<String> {
        Some(match self {
            Action::Nothing => return None,
            Action::Pad(b) => format!("xinput_button {}", b.steam()),
            Action::Key(k) => format!("key_press {}", k.to_ascii_uppercase()),
            Action::Mouse(m) => format!("mouse_button {}", m.steam()),
            Action::WheelUp => "mouse_wheel SCROLL_UP".into(),
            Action::WheelDown => "mouse_wheel SCROLL_DOWN".into(),
            Action::Steam(SteamAction::Screenshot) => "controller_action SCREENSHOT".into(),
            Action::Steam(SteamAction::ShowKeyboard) => "controller_action SHOW_KEYBOARD".into(),
            Action::Steam(SteamAction::SetLed { r, g, b, rest }) => {
                if rest.is_empty() {
                    format!("controller_action set_led {r} {g} {b}")
                } else {
                    format!("controller_action set_led {r} {g} {b} {rest}")
                }
            }
            Action::Steam(SteamAction::ChangePreset(args)) => format!("controller_action CHANGE_PRESET {args}"),
            Action::Other(s) => s.clone(),
        })
    }

    /// The full binding value to write: Steam's save style `"<command>, , "`. When the old binding had the same command its
    /// own label / icon fields are kept byte for byte.
    pub fn binding_value(&self, old: Option<&str>) -> Option<String> {
        let cmd = self.command()?;
        if let Some(old) = old {
            if let Some(i) = old.find(',') {
                if old[..i].trim() == cmd {
                    return Some(old.to_string());
                }
            } else if old.trim() == cmd {
                return Some(old.to_string());
            }
        }
        Some(format!("{cmd}, , "))
    }

    /// The light-bar red the drawing offers (exactly a real Rocket League binding).
    pub fn light_bar_red() -> Action {
        Action::Steam(SteamAction::SetLed { r: 255, g: 0, b: 0, rest: "0 0 1".into() })
    }

    /// How the drawing shows it.
    pub fn label(&self, xbox: bool) -> String {
        match self {
            Action::Nothing => "Nothing".into(),
            Action::Pad(b) => (if xbox { b.xbox_name() } else { b.ps_name() }).into(),
            Action::Key(k) => key_label(k),
            Action::Mouse(m) => m.label().into(),
            Action::WheelUp => "Wheel up".into(),
            Action::WheelDown => "Wheel down".into(),
            Action::Steam(SteamAction::Screenshot) => "Screenshot".into(),
            Action::Steam(SteamAction::ShowKeyboard) => "Show keyboard".into(),
            Action::Steam(SteamAction::SetLed { r: 255, g: 0, b: 0, .. }) => "Light bar red".into(),
            Action::Steam(SteamAction::SetLed { r, g, b, .. }) => format!("Light bar #{r:02x}{g:02x}{b:02x}"),
            Action::Steam(SteamAction::ChangePreset(_)) => "Switch action set".into(),
            Action::Other(s) => s.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_every_measured_form() {
        assert_eq!(Action::parse("xinput_button A, , "), Action::Pad(PadButton::Cross));
        assert_eq!(Action::parse("xinput_button dpad_up, , "), Action::Pad(PadButton::DpadUp));
        assert_eq!(Action::parse("xinput_button DPAD_UP"), Action::Pad(PadButton::DpadUp));
        assert_eq!(Action::parse("xinput_button shoulder_left, , "), Action::Pad(PadButton::L1));
        assert_eq!(Action::parse("xinput_button SELECT, , "), Action::Pad(PadButton::Create));
        assert_eq!(Action::parse("xinput_button JOYSTICK_RIGHT"), Action::Pad(PadButton::R3));
        assert_eq!(Action::parse("key_press F5, , "), Action::Key("F5".into()));
        assert_eq!(Action::parse("mouse_button LEFT, Fire, "), Action::Mouse(MouseButton::Left));
        assert_eq!(Action::parse("mouse_wheel SCROLL_DOWN, , "), Action::WheelDown);
        assert_eq!(Action::parse("controller_action SCREENSHOT, , "), Action::Steam(SteamAction::Screenshot));
        assert_eq!(Action::parse("controller_action set_led 255 0 0 0 0 1, , "), Action::light_bar_red());
        assert_eq!(Action::parse("game_action ui ui_pause, Pause, "), Action::Other("game_action ui ui_pause".into()));
        assert_eq!(Action::parse(""), Action::Nothing);
    }

    #[test]
    fn writes_steam_save_style_and_keeps_an_unchanged_binding() {
        assert_eq!(Action::Key("F5".into()).binding_value(None).unwrap(), "key_press F5, , ");
        assert_eq!(Action::light_bar_red().binding_value(None).unwrap(), "controller_action set_led 255 0 0 0 0 1, , ");
        // same command → the old label fields are kept byte for byte
        assert_eq!(Action::Mouse(MouseButton::Left).binding_value(Some("mouse_button LEFT, Fire, icon")).unwrap(), "mouse_button LEFT, Fire, icon");
        // a new command → Steam's empty fields
        assert_eq!(Action::Pad(PadButton::Circle).binding_value(Some("xinput_button A, , ")).unwrap(), "xinput_button B, , ");
        assert_eq!(Action::Nothing.binding_value(None), None);
        for b in PadButton::ALL {
            assert_eq!(Action::parse(&Action::Pad(b).binding_value(None).unwrap()), Action::Pad(b));
        }
        for m in MouseButton::ALL {
            assert_eq!(Action::parse(&Action::Mouse(m).binding_value(None).unwrap()), Action::Mouse(m));
        }
    }

    #[test]
    fn key_names() {
        assert_eq!(key_token("Shift").as_deref(), Some("LEFT_SHIFT"));
        assert_eq!(key_token("q").as_deref(), Some("Q"));
        assert_eq!(key_token("Page Down").as_deref(), Some("PAGE_DOWN"));
        assert_eq!(key_label("LEFT_CONTROL"), "Ctrl");
        assert_eq!(key_label("ENTER"), "Enter");
        assert_eq!(Action::Key("SPACE".into()).label(false), "Space");
        assert_eq!(Action::Pad(PadButton::L1).label(true), "LB");
    }
}
