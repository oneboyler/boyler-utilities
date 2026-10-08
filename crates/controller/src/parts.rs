//! The controllers the Controller tab draws and where each of their parts lives in a Steam layout.

/// The controller types the tab supports (owner feedback on v20: "at least ps4, ps5, xbox, edge").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PadKind {
    DualSense,
    DualSenseEdge,
    DualShock4,
    /// Xbox One / Series (also Elite).
    Xbox,
}

impl PadKind {
    pub const ALL: [PadKind; 4] = [PadKind::DualSenseEdge, PadKind::DualSense, PadKind::DualShock4, PadKind::Xbox];

    /// The `<type>` in `controller_<type>.vdf` / `configset_controller_<type>.vdf` (measured: the Edge uses `ps5`; a
    /// Rocket League `controller_ps5.vdf` says `controller_type controller_ps5_edge`).
    pub fn layout_type(self) -> &'static str {
        match self {
            PadKind::DualSense | PadKind::DualSenseEdge => "ps5",
            PadKind::DualShock4 => "ps4",
            PadKind::Xbox => "xboxone",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            PadKind::DualSense => "DualSense",
            PadKind::DualSenseEdge => "DualSense Edge",
            PadKind::DualShock4 => "DualShock 4",
            PadKind::Xbox => "Xbox Wireless Controller",
        }
    }

    pub fn has_gyro(self) -> bool {
        !matches!(self, PadKind::Xbox)
    }
    pub fn has_touchpad(self) -> bool {
        !matches!(self, PadKind::Xbox)
    }
    pub fn has_light_bar(self) -> bool {
        !matches!(self, PadKind::Xbox)
    }
    pub fn is_xbox(self) -> bool {
        matches!(self, PadKind::Xbox)
    }

    /// Which buttons this pad has.
    pub fn buttons(self) -> Vec<ButtonId> {
        ButtonId::ALL.into_iter().filter(|b| b.on(self)).collect()
    }

    /// From USB vendor / product id. Sources: the product ids Steam / SDL / Linux hid-playstation use (not measured here
    /// except the Edge: `VID_054C&PID_0DF2`).
    pub fn from_ids(vid: u16, pid: u16) -> Option<PadKind> {
        match (vid, pid) {
            (0x054C, 0x0CE6) => Some(PadKind::DualSense),
            (0x054C, 0x0DF2) => Some(PadKind::DualSenseEdge),
            (0x054C, 0x05C4) | (0x054C, 0x09CC) | (0x054C, 0x0BA0) => Some(PadKind::DualShock4),
            (0x045E, p) if XBOX_PIDS.contains(&p) => Some(PadKind::Xbox),
            _ => None,
        }
    }
}

/// Xbox One / Series / Elite product ids (USB + Bluetooth), Microsoft vendor 0x045E.
pub const XBOX_PIDS: &[u16] = &[0x02D1, 0x02DD, 0x02E0, 0x02E3, 0x02EA, 0x02FD, 0x02FF, 0x0B00, 0x0B02, 0x0B05, 0x0B12, 0x0B13, 0x0B20, 0x0B21, 0x0B22];

/// Left or right (sticks, triggers, touchpad halves).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Left,
    Right,
}

/// Every pressable button the tab shows. The four back slots are Steam's (`button_back_left(_upper)` …).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ButtonId {
    Cross,
    Circle,
    Square,
    Triangle,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
    L1,
    R1,
    L3,
    R3,
    Create,
    Options,
    /// PS / Xbox button — Steam keeps it for its own menu (shown, never written).
    Home,
    /// PS5 Mute / Xbox Series Share (Steam: `button_capture`; a real file: Screenshot).
    Mute,
    BackLeftUpper,
    BackLeftLower,
    BackRightUpper,
    BackRightLower,
}

/// Where a part's binding lives: the group SOURCE in the action set + the INPUT inside that group (+ the mode Steam
/// uses for a new group of that source).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Place {
    pub source: &'static str,
    pub input: &'static str,
    pub new_mode: &'static str,
}

/// The four directions of a four-button or d-pad group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dir {
    N,
    S,
    E,
    W,
}

fn dir_input(mode: &str, d: Dir) -> &'static str {
    // a `four_buttons` group names its inputs button_a/b/x/y, a `dpad` group dpad_north/… (measured)
    if mode == "four_buttons" {
        match d {
            Dir::S => "button_a",
            Dir::E => "button_b",
            Dir::W => "button_x",
            Dir::N => "button_y",
        }
    } else {
        match d {
            Dir::N => "dpad_north",
            Dir::S => "dpad_south",
            Dir::E => "dpad_east",
            Dir::W => "dpad_west",
        }
    }
}

impl ButtonId {
    pub const ALL: [ButtonId; 20] = [
        ButtonId::Cross,
        ButtonId::Circle,
        ButtonId::Square,
        ButtonId::Triangle,
        ButtonId::DpadUp,
        ButtonId::DpadDown,
        ButtonId::DpadLeft,
        ButtonId::DpadRight,
        ButtonId::L1,
        ButtonId::R1,
        ButtonId::L3,
        ButtonId::R3,
        ButtonId::Create,
        ButtonId::Options,
        ButtonId::Home,
        ButtonId::Mute,
        ButtonId::BackLeftUpper,
        ButtonId::BackLeftLower,
        ButtonId::BackRightUpper,
        ButtonId::BackRightLower,
    ];

    /// Does this pad have the button?
    pub fn on(self, kind: PadKind) -> bool {
        match self {
            ButtonId::BackLeftUpper | ButtonId::BackLeftLower | ButtonId::BackRightUpper | ButtonId::BackRightLower => kind == PadKind::DualSenseEdge,
            ButtonId::Mute => kind != PadKind::DualShock4,
            _ => true,
        }
    }

    /// Steam keeps the PS / Xbox button for its own menu.
    pub fn is_fixed(self) -> bool {
        self == ButtonId::Home
    }

    /// The source + input of this button, for a group in `mode` (the face / d-pad inputs depend on the group's mode).
    pub fn place(self, mode: Option<&str>) -> Option<Place> {
        let face = |d: Dir| {
            let m = mode.unwrap_or("four_buttons");
            Place { source: "button_diamond", input: dir_input(m, d), new_mode: "four_buttons" }
        };
        let dpad = |d: Dir| {
            let m = mode.unwrap_or("dpad");
            Place { source: "dpad", input: dir_input(m, d), new_mode: "dpad" }
        };
        let sw = |input: &'static str| Place { source: "switch", input, new_mode: "switches" };
        Some(match self {
            ButtonId::Cross => face(Dir::S),
            ButtonId::Circle => face(Dir::E),
            ButtonId::Square => face(Dir::W),
            ButtonId::Triangle => face(Dir::N),
            ButtonId::DpadUp => dpad(Dir::N),
            ButtonId::DpadDown => dpad(Dir::S),
            ButtonId::DpadLeft => dpad(Dir::W),
            ButtonId::DpadRight => dpad(Dir::E),
            ButtonId::L1 => sw("left_bumper"),
            ButtonId::R1 => sw("right_bumper"),
            ButtonId::Create => sw("button_menu"),
            ButtonId::Options => sw("button_escape"),
            ButtonId::Mute => sw("button_capture"),
            ButtonId::BackLeftUpper => sw("button_back_left_upper"),
            ButtonId::BackLeftLower => sw("button_back_left"),
            ButtonId::BackRightUpper => sw("button_back_right_upper"),
            ButtonId::BackRightLower => sw("button_back_right"),
            ButtonId::L3 => Place { source: "joystick", input: "click", new_mode: "joystick_move" },
            ButtonId::R3 => Place { source: "right_joystick", input: "click", new_mode: "joystick_move" },
            ButtonId::Home => return None,
        })
    }

    /// The source whose group mode decides the input names.
    pub fn source(self) -> Option<&'static str> {
        self.place(None).map(|p| p.source)
    }

    /// The name the pad prints on it.
    pub fn name(self, kind: PadKind) -> &'static str {
        let xb = kind.is_xbox();
        match self {
            ButtonId::Cross => if xb { "A" } else { "Cross" },
            ButtonId::Circle => if xb { "B" } else { "Circle" },
            ButtonId::Square => if xb { "X" } else { "Square" },
            ButtonId::Triangle => if xb { "Y" } else { "Triangle" },
            ButtonId::DpadUp => "D-pad up",
            ButtonId::DpadDown => "D-pad down",
            ButtonId::DpadLeft => "D-pad left",
            ButtonId::DpadRight => "D-pad right",
            ButtonId::L1 => if xb { "LB" } else { "L1" },
            ButtonId::R1 => if xb { "RB" } else { "R1" },
            ButtonId::L3 => if xb { "LS press" } else { "L3" },
            ButtonId::R3 => if xb { "RS press" } else { "R3" },
            ButtonId::Create => match kind {
                PadKind::Xbox => "View",
                PadKind::DualShock4 => "Share",
                _ => "Create",
            },
            ButtonId::Options => if xb { "Menu" } else { "Options" },
            ButtonId::Home => if xb { "Xbox button" } else { "PS button" },
            ButtonId::Mute => if xb { "Share" } else { "Mute button" },
            // Steam Deck naming for its four back slots; on the Edge which slot is the paddle and which the Fn button is
            // UNCLEAR (Steam's own gyro list pairs the Edge's Fn buttons with the Deck's L4/R4 values and the paddles with
            // L5/R5 — so "upper" = Fn is the guess). A real Rocket League F5 sits on button_back_right_upper.
            ButtonId::BackLeftUpper => "L4",
            ButtonId::BackLeftLower => "L5",
            ButtonId::BackRightUpper => "R4",
            ButtonId::BackRightLower => "R5",
        }
    }
}

/// Every part the picture shows (buttons + the parts with their own panels).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Part {
    Button(ButtonId),
    Stick(Side),
    Trigger(Side),
    Gyro,
    Touchpad,
}

impl Side {
    pub fn stick_source(self) -> &'static str {
        match self {
            Side::Left => "joystick",
            Side::Right => "right_joystick",
        }
    }
    pub fn trigger_source(self) -> &'static str {
        match self {
            Side::Left => "left_trigger",
            Side::Right => "right_trigger",
        }
    }
    pub fn trackpad_source(self) -> &'static str {
        match self {
            Side::Left => "left_trackpad",
            Side::Right => "right_trackpad",
        }
    }
}
