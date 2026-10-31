//! bu-gpudriver — the NVIDIA graphics driver behind the Display tab's "Graphics driver" card (Order 125). No UI.
//!
//! Detect the NVIDIA card + its driver (Windows' own adapter list), look up NVIDIA's newest and older Game Ready
//! drivers for it, download one with progress, check it is signed by NVIDIA, and install the DRIVER ONLY (no NVIDIA app,
//! no GeForce Experience, no telemetry) - the install runs in the app's elevated copy (one admin prompt). Nothing runs in
//! the background: every lookup is one call the card makes when it is shown or refreshed.
//!
//! The OS sits behind traits ([`GpuOs`], [`Signatures`], [`InstallOs`]); `real` holds the Windows ones, `fake` the test ones.
//! NVIDIA's network answers go through `bu_updater::Http` (WinHTTP; its fake in tests).

pub mod detect;
pub mod download;
pub mod error;
pub mod fake;
pub mod install;
pub mod lookup;
#[cfg(windows)]
pub mod real;
pub mod version;

pub use detect::{detect, Adapter, GpuOs, NvidiaGpu};
pub use download::{check_nvidia, download, Signatures};
pub use error::{DriverError, Result};
pub use install::{install, InstallOs, Installed};
pub use lookup::{lookup, Release};
pub use version::DriverVersion;
