//! Add-ons (Order 037, the owner Oct 8: "add ons page just provides quicker way to see all thats downloaded add ons wise"):
//! only things you download. This crate is the download / install part of the one that is really downloaded today -
//! **Mouse acceleration** = Raw Accel's official release:
//! - the release is PINNED (version, link, size, SHA-256 - [`rawaccel::PIN`]); a download that differs in one byte is
//!   thrown away, never unpacked (never a changed driver);
//! - unpacked with [`zip`] into a staging folder (only plain names inside `RawAccel/`, nothing outside it), then the
//!   driver's Authenticode signature is checked by Windows (WinVerifyTrust) before anything runs;
//! - installed / removed by Raw Accel's OWN installer.exe / uninstaller.exe (they need admin and a restart), run through
//!   ONE elevated helper ([`helper`], our own exe started with Windows' admin prompt) that checks the tool's and the
//!   driver's SHA-256 again with the files held against writes, runs the tool hidden and answers its "Press any key";
//! - every OS step is behind [`os::AddonOs`]: [`real::RealOs`] on Windows, [`fake::FakeOs`] for the tests (the tests never
//!   download, install or remove anything for real).
//!
//! The page add-on (Notifications for OBS) is built into the app (Order 035) and needs nothing from here.

pub mod error;
pub mod fake;
pub mod os;
pub mod rawaccel;
pub mod zip;

#[cfg(windows)]
pub mod helper;
#[cfg(windows)]
pub mod real;

pub use error::{AddonError, Result};
pub use os::{AddonOs, Elevated, HelperAction};
pub use rawaccel::{RaState, Step};

/// SHA-256 of `data` as lowercase hex (the updater's own implementation).
pub fn sha256_hex(data: &[u8]) -> String {
    bu_updater::sha256::sha256_hex(data)
}
