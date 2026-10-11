# Boyler Utilities — What's new

## 1.0.6 (2026-10-10)
- Keyboard tab: the keyboard comes first; a folding Keyboard sounds card. Click any key: what it does (Normal / Remap / Action / Macro with real steps - key down, key up, press, type, wait, click, open) and its Sound - the pack's sound on or off for that key plus your own sound on top (press / release file, pitch, loudness).
- Pick many keys at once (drag a box, Ctrl, Shift) and give them one sound: pitch and loudness Same or Random, while Space, Enter, Shift, Ctrl and the other big keys keep their own deeper shape.
- "Make a pack from one sound": one file becomes a whole pack. Ignore repeats now goes up to 400 ms. Keyboards the app knows pick their size (Full / TKL / 75 % / 60 %) by themselves.
- Mouse tab: your mouse drawn on the left - click any button to give it an action, a macro or a sound; DPI 400 / 800 / 1600 / Custom (type your own). Keyboards with a mouse function (like Wooting) no longer show up as mice.
- Mouse acceleration: a new card starts from your Raw Accel curve, and the sliders cover Raw Accel's full range.
- Controller tab: macros on any button (written into the game's Steam layout, so they work in games), a sound per button, Button sounds in Controller settings; the light colour really reaches the pad (Restart Steam to apply).
- Audio: click the Output speaker to mute the PC. The mic-mute sound's Volume sits right under its switch.
- Notifications for OBS: no more false "can't save" before a slow clip saves; the tray icon shows which monitor OBS records.
- Noise: soft waves while it plays and a quiet line of how long you listened (today, week, month, year, all time).
- A broken sound pack can no longer crash the app.

## 1.0.5 (2026-10-09)
- The menu no longer blinks: switching between the GPU and CPU drawing path kept the window, glass included, instead of rebuilding it empty. Hidden Windows windows (emoji panel, clipboard) no longer count as a game in front.
- Keyboard tab: the sound you pick stays picked (a finished download no longer takes it back). New "Play on": press + release, press only or release only, for keys and mouse.
- Keyboard tab: much less text; "Different in some apps" is part of the Sound card; one "Import a pack…"; Get more sounds lists every pack of mechvibes.com with a search box and a play button per pack, and shows the one in use; right-click a pack you added to remove it.
- Mouse acceleration: never overwrites a newer change made in Raw Accel itself ("Use ours again" puts it back); a listed game that is already running gets its preset at once; Linear starts from 2.6 / cap 2.0 / offset 55.
- Timers: the big stopwatch / countdown belongs to the tab; "Your timers" lists only the timers you add. World clock: search any of 33,000+ cities.
- Noise: your own mix - Tone, Rumble and Waves sliders, live while playing, and "Save as my sound".
- Startup: Store apps (Spotify, Xbox, Claude…) open Windows Settings › Apps › Startup straight from their switch; Xbox, Windows Terminal and Phone Link no longer count as parts of Windows.

## 1.0.4 (2026-10-09)
- Mouse tab: every mouse Windows lists is now found and named - by its model where known (Pulsar X2 V2 / X2A Wireless, Logitech, ASUS ROG, SteelSeries, Roccat, Glorious ...), else by the name Windows gives it - with its VID:PID shown, and the brand's settings link. Extra mice show as "Also connected".
- Keyboard tab: Sound › Get more sounds lists the community packs of mechvibes.com with their size; one click downloads and imports one.
- Keyboard tab: the keyboard picture is now the first thing on the tab; keys that carry a remap, an action or a macro glow blue. Clicking a key opens a small window of its own (Normal / Remap / Action / Macro).
- Many more ready-made actions for a key: copy, paste, cut, undo, redo, select all, Alt + Tab, Task Manager, lock the PC, show the desktop, emoji panel, snipping tool, browser back / forward / refresh, new / close / reopen tab, media, volume, switch audio output, open an app / folder / website. Three ready-made macros to start from (type my e-mail, open 3 sites, copy + search Google).
- Importing a Mechvibes sound pack now takes the .zip exactly as downloaded (or a folder), and says plainly when it is not a pack.
- New "Ignore repeats within" setting for key sounds (0-80 ms, off by default) for keyboards that register one press twice. It only quiets the sounds.
- "Off while a game is in front" is now off by default (a setting you already saved stays as it is).
- Better key sounds: layered (a click, a damped body, a soft case tail), deeper Space / Enter, a lighter key-up, and no harsh ping in the satisfying sounds.

- Keyboard: a key set to press itself (J -> J) no longer gets stuck; mouse click sounds (off by default; side buttons use the keyboard pack); "Play on" press / release / both.
- Voice to text: the mic opens Windows' own voice typing (the same as Win + H) - no privacy setting to turn on.
- New Noise tab: white, pink, brown, dark brown, grey and blue noise that loops in the background (fade, sleep timer, "Stop noise" in the tray) - about 0 % CPU.
- Mouse acceleration: saved and back after a restart, per-game presets really switch (Display's per-game rules too), and it notices when Raw Accel's own app overwrites it.
- Cursors: no more "SPI_SETCURSORS failed", every set shows its real cursor, the size slider works, your own cursors are listed, and "Get more cursors" (19 free packs).
- Storage: measures with Everything (seconds instead of a long walk), "Measure again" after any clean, shader / launcher caches unticked by default, cleaning launcher caches keeps you logged in, and a Folders | Files switch for the 20 biggest files.
- The open menu stays in front of other windows (it steps back only for a fullscreen game).
- Tweaks: every row checked against Windows' own Settings; the lock-screen tips row no longer changes your lock screen picture.

## 1.0.3 (2026-10-09)
- New Keyboard tab: a picture of your keyboard - click any key to remap it, give it an action (media, volume, mic mute, screenshot, open an app or website) or a macro.
- Key sounds (off until you turn them on): clean keyboard sounds and satisfying ones, quiet by default, silent while a game is in front. The app only uses the key to pick the sound and forgets it at once - nothing is stored.

## 1.0.2 (2026-10-09)
- Changing the resolution live: the menu and its Keep / Revert bar now stay on the monitor you changed instead of jumping to your other screen.

## 1.0.1 (2026-10-09)
- Never freezes your PC: the screenshot overlay can no longer lock the desktop, and slow Windows work (changing a
  setting, Launch Steam, reading devices) no longer holds up the menu.
- Lighter while you play: a gaming mouse moving no longer wakes the app, and per-game display / mouse switching no longer
  makes Windows' WMI service poll every second.
- The menu and the screenshot overlay draw on the graphics card, and only when something changes.
- File search uses less memory, and Setup's Everything install no longer starts indexing for every user at once.

## 1.0.0 (2026-10-08)
- First release: one glass menu in your tray with 18 tabs for the Windows settings that are slow to find.
- Volume for every device and app, mic mute on one key, display presets that switch on by themselves for a game.
- Screenshots with drawing tools, your own cursors and mouse acceleration, Steam controller buttons.
- Dozens of hidden Windows switches, startup apps, live performance, a speed test and safe storage clean-up.
- Fast file search, voice to text, timers and screen time, with nothing leaving your PC.
- Dark or light glass, and one undo for everything the app changed.
