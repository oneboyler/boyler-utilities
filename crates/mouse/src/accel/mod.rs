//! "Mouse acceleration" = Raw Accel (DESIGN §3.4; decided: the app ships nothing of Raw Accel and never changes its
//! driver — it reads the user's installed Raw Accel and hands its driver settings the way Raw Accel itself does).
//!
//! - [`args`]: Raw Accel's settings (structs, defaults, settings.json names) — v1.7.0.
//! - [`curves`]: its maths, ported exactly (every curve, gain + legacy, caps, the per-packet modifier, validation).
//! - [`bytes`]: the driver's binary layout (WRITE / READ).
//! - [`panel`]: the card — curves, values, presets, the mapping onto Raw Accel's args.
//! - [`switch`]: per-app rows + "Everywhere else", the start/stop planning, the settle delay.
//! - [`service`]: `Mouse<O>` methods tying it together (status, mirror, copy its curve, sync the driver, header line, graph).

pub mod args;
pub mod bytes;
pub mod curves;
pub mod panel;
pub mod persist;
pub mod service;
pub mod switch;
