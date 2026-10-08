# Building Boyler Utilities

## What you need

- Windows 10 or 11, 64-bit
- [Rust](https://rustup.rs) (stable, MSVC toolchain) with clippy
- Visual Studio 2022 Build Tools with "Desktop development with C++" (the MSVC linker and Windows SDK)
- Internet on the first build: the drawing library (Skia, through the `skia-safe` crate) downloads its prebuilt binaries
- For the installer only: [Inno Setup 6](https://jrsoftware.org/isinfo.php) and Git Bash

## Build and run

```
cargo build --release -p bu-app
```

The app is `target\release\BoylerUtilities.exe`. Run it and double-click the tray icon.

## Test

```
cargo test --workspace
cargo clippy --workspace -- -D warnings
```

Or the quiet version that prints only errors, warnings and failing tests (Git Bash):

```
tools/check.sh -p bu-app -p bu-audio
```

The tests use fake versions of Windows (every feature crate has a REAL and a FAKE layer behind one trait), so they never change your real devices, volumes or settings. A few tests write into a scratch folder (`C:\BoylerUtilities-scratch`) or a scratch registry key (`HKCU\Software\BoylerUtilities-test`) and remove them again; tests that need something that isn't there are skipped.

## The installer

```
tools/installer/build.sh
```

This builds the release exe and then `dist\Boyler Utilities Setup.exe` with Inno Setup (per user, no admin).

## Where things are

- `app/`: the tray app itself, meaning the glass menu, its frame and one page per tab (`app/src/pages/`), drawn with Skia.
- `crates/<area>/`: one library per feature area (audio, display, mouse, network …), with no UI. Each has its Windows layer behind a trait plus a fake one for tests, and many have an `examples/show.rs` that prints the real current state read-only (`cargo run -p bu-display --example display-show`).
- `tools/`: the check script, the installer script, the third-party licence list generator, and the cursor generator.
