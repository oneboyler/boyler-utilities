//! bu-screenshot — the Screenshots engine (DESIGN.md §3.3). No UI and no overlay: the overlay and the Screenshots tab only draw
//! what this crate hands them and send it commands.
//!
//! - [`service::Screenshots`]: capture (all monitors frozen in one go, one monitor, a region, Live), output (clipboard PNG + DIB,
//!   PNG into the chosen folder, file naming), the gallery (index, thumbnails, delete to the Recycle Bin, show in folder,
//!   drag-out files).
//! - [`geom`]: monitors in physical pixels and the overlay's pure math (monitor under the mouse, typed sizes, lit presets).
//! - [`image`], [`encode`]: the pixels (crop, compose, rotation, HDR → SDR, thumbnails) and the bytes (PNG, DIB).
//! - [`os::ScreenshotOs`]: the OS layer — [`real::RealOs`] (Windows: DXGI Desktop Duplication + Windows.Graphics.Capture,
//!   clipboard, shell) and [`fake::FakeOs`] (tests).
//!
//! No keys are registered here (wave-2 rule): the app layer maps the Screenshot key to [`service::Screenshots::capture_all`].

pub mod encode;
pub mod error;
pub mod fake;
pub mod gallery;
pub mod geom;
pub mod image;
pub mod naming;
pub mod os;
#[cfg(windows)]
pub mod real;
pub mod service;

pub use error::{Error, Result};
pub use gallery::Shot;
pub use geom::{Monitor, Rect, Rotation};
pub use image::Image;
pub use os::{Capture, CaptureTiming, ColorPath, Method, MonitorFrame, ScreenshotOs};
pub use service::{Frozen, Live, Screenshots, Target, DEFAULT_METHOD};
