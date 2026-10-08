//! `bu-search` — the Search tab's features (DESIGN.md §3.15), no UI: "like the old Windows search" for the whole PC.
//! Only what is on this PC: no web results, no ads, no suggestions.
//!
//! * Apps come from `shell:AppsFolder` (desktop programs and Store apps).
//! * Folders and files come from **Everything** when its SDK DLL is next to the app and Everything runs, else from
//!   **Windows Search's own index** (no admin either way). [`SearchService::backend_report`] says which, and which
//!   folders the Windows Search index covers. No own whole-disk indexer yet (it needs an admin helper: a later order).
//! * [`SearchService::search`] ranks by name (starts with the text > a word starts with it > anywhere, then shorter
//!   names), groups Apps · Folders · Files and caps them (4 / 4 / 6 under All, 40 under one chip).
//! * The right-click menu and Open: [`SearchService::menu_for`], [`SearchService::run_menu`], [`SearchService::open`].
//!
//! Nothing runs in the background and nothing is loaded until the first word is typed; [`SearchService::release`] drops
//! it all when the menu closes. Every Windows call goes through the [`SearchOs`] trait: [`RealOs`] and [`FakeOs`].

mod error;
pub mod fake;
pub mod model;
mod os;
#[cfg(windows)]
pub mod real;
mod service;

pub use error::{Result, SearchError};
pub use fake::FakeOs;
pub use model::*;
pub use os::*;
#[cfg(windows)]
pub use real::RealOs;
pub use service::*;
