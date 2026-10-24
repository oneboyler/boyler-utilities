# Boyler Utilities — What's new

## 1.2.1 (2026-10-24)
- Notes: the text of a note now works like a normal text box. Drag the mouse to select, double-click a word, Shift+arrows (and Shift+click) extend the selection, Ctrl+Arrows jump by word, Ctrl+A selects all, Ctrl+C copies, Ctrl+X cuts, Ctrl+V pastes over the selection; the same in a note's title. The selection shows as the app's blue with white letters on every paper colour. Before, nothing could be selected or copied.
- Right-click a note: "Copy" (the selection, or the whole title / text when nothing is selected) and "Paste" now sit between the colours and "Delete note".
- Selecting: Ctrl / Shift / a letter no longer drops the selection (so Ctrl+C copies and a typed letter replaces it), and a double-clicked word stays whole when the mouse moves a hair.
- Blurred notes: the real letters are no longer drawn at all - a blurred note shows random stand-in letters in the same lines (new ones only when its text changes), blurred less, so you can see there is text but nothing can be read or worked back. Titles stay sharp; click a note to see its real text.
- The licence list in Settings › About now matches the libraries the app is built with (it listed two old versions and missed one library).
- Controller: a stick's Dead zone and Full at now show the same numbers as Steam's own Deadzones panel, and a number you set here is the number Steam shows. Steam measures a stick's dead zone out of 32000, not 32767, so before, 86 % here was 88 % in Steam; and a layout without its own value now shows Steam's real starting values (dead zone 0 %, full at 88 %, shape Circle, source "This controller's own") instead of 8 % / 100 % / Custom. Setting a dead zone or full at here also switches the stick's Source to Custom (Steam only uses the numbers then), and the value is always written into the layout - setting it to 8 % used to remove the line, and Steam kept showing its own old number. Trigger dead zones start at Steam's own 3 % / 98 %.
- Controller: the stick's dead zone picture now looks like Steam's - solid grey where the stick gives no output, light blue where the output ramps up, dark blue where it is full - and takes the dead zone's shape: Circle (discs), Cross (a grey cross inside the circle) or Square (square zones with a grey cross). It changes at once with the sliders, the shape and the source; the live dot of your stick stays. With Source "This controller's own" it shows the controller's own dead zone like Steam does, also when the controller has never been calibrated (Steam's default for the type, e.g. 31 % for a DualSense), and the response curve and the "out" number follow the source.

## 1.2.0 (2026-10-24)
- Audio: right-click the Input slider › "Lock at <value>" keeps the microphone volume there. While it is locked the slider is red with "Locked - Windows can't change it" under it, and any change of the default microphone's volume - by Windows or an app - is put back at once (no polling). It follows a change of default microphone, is saved and holds after a restart. Right-click again › "Unlock". Input only.
- Mouse tab: the left / right button highlight no longer shows under the scroll wheel.
- Nothing the app starts (or its tests) opens a console window any more.
- The file properties of BoylerUtilities.exe (Details tab) now show the real version instead of 1.0.4.
- New Notes tab (between Screenshots and Mouse): sticky notes you type straight into, in tabs like a browser's - "+" adds a tab, double-click renames it, right-click deletes it (it asks first when the tab has notes). Each note has a title and its text, "+ Note" adds one, right-click a note to delete it. Everything is saved as you type and is still there after a restart (its own file, notes.cfg, next to the settings - "Reset the app's own settings" never touches your notes). Ctrl+V pastes.
- The eye at the top right of Notes blurs the text of every note (the titles stay readable); click a note to see it until you leave the tab. Right-click the eye: "Blur every time I open this tab".
- Right-click any icon of the top row: "Rearrange tabs" (drag the icons where you want them, click anywhere else when done) or "Reset order". The order is remembered; tabs that later versions add take their own place in it.
- Screenshot overlay feels snappier: the crosshair is now a real Windows pointer, so it moves with the mouse instead of a frame behind it; the overlay keeps up with your monitor's refresh rate even while the graphics card idles at its low clocks (it was held to about 220 frames a second on a 360 Hz screen); a monitor the pointer isn't on is no longer redrawn on every mouse move, and nothing is redrawn while nothing moves; and the overlay opens a little sooner (its graphics are prepared while the screen is being grabbed).
- Screenshot overlay: click the width or the height of the size tag to type just that number in its own small box.
- Screenshots: the eye left of "Change path" blurs every thumbnail. Right-click it: "Blur every time I open this tab".
- Notes now look like real sticky notes: square pastel paper (yellow, pink, blue, green - each new note takes the next colour), dark text, a soft shadow and a slight tilt of its own, three to a row, and "+ Note" is a dashed square. The eye still blurs only the text; typing works on the paper as before. Notes made before keep their text and get the colours in turn.
- Right-click a note: a small row of the four paper colours (the note's own is ringed) above "Delete note" - click one and that paper changes colour at once; it is saved with the note.
- The top row shows its first 15 icons whole again, through Search (Notes had pushed Search out of sight): the gaps between the icons are a little tighter, the icons themselves are the same size. The rest are still behind the row's scroll. Rearranged rows work the same.

## 1.1.1 (2026-10-13)
- Mouse tab: five more settings of the mouse itself, under Lift-off distance - Debounce time (− / +), Motion Sync, Angle Snap, Ripple Control and Turbo Mode. Each mouse shows only the ones its driver can really change: Pulsar, ATK / VXE, Endgame Gear, Lamzu, WLMouse, Lingbao / Attack Shark (GearHub), Corsair, Teevolution, VGN, Ninjutso, Finalmouse, Zaunkoenig, Orbital, RAWM, Delux, ASUS, Incott, WALLHACK, MCHOSE and Keychron M6 (Turbo Mode on Pulsar, Teevolution, VGN and WLMouse); a mouse without them looks as before. Every change is saved on the mouse and read back.
- Lift-off distance: 0.7 mm on the mice that have it (Pulsar and others).

## 1.1.0 (2026-10-13)
- Boyler Utilities is now free software under the GNU AGPL v3 (or later) - see Settings › Licences. Its mouse drivers are ported from OpenMouse (credited there).
- Mouse tab: DPI, polling rate and battery (and lift-off distance on Pulsars) now work for many more mice - Pulsar (also the Tenz Signature, X2h, X2 V3 ES Mini, X2F and X3 Medium), Logitech (37 models, cabled or on a Lightspeed / Bolt receiver), Razer (14 models) and SteelSeries (10 models: polling rate and battery; DPI on the Rival 650 and Rival 310 only - on the others a DPI change from here would erase the presets saved on the mouse). Each mouse shows only the settings it supports. Opening the tab only reads the mouse; a setting changes only when you click it.
- Pulsar X2 CrazyLight: works as before (now with exactly the requests OpenMouse sends).
- The Screenshot key works again while a game or a full-screen window is in front (1.0.9 made it do nothing there).
- Mouse tab, more brands (all from OpenMouse's supported list): ATK / VXE, Endgame Gear, Lamzu (also CRDRAKO, LunaFury, Attack Shark R5 Ultra), WLMouse, VAXEE, Corsair, Redragon, Attack Shark / Lingbao (GearHub), Glorious, Ninjutso, Finalmouse, Zaunkoenig, RAWM, Mchose, Delux, Dareu, Keychron, Incott, K-snake, Microsoft, moddoMOUSE, Teevolution, Wallhack, VGN, ASUS, Orbital and Rapoo - roughly 70 more models, well over 130 in all. Each mouse shows only what its driver can really do (DPI, polling rate, lift-off distance, battery); where changing a setting from here would overwrite other settings saved on the mouse, that setting is left to the brand's own software.

## 1.0.9 (2026-10-12)
- Controller settings: Left / Right stick dead zone are back and now apply LIVE - they change the controller profile of the game picked at the top of the Controller tab (Steam picks it up when you click back into the game).
- Every change to a game's controller profile is backed up first. If Steam (or its cloud copy) puts other values back, the tab tells you and offers "Put mine back".
- Screenshots: dragging a screenshot into Photoshop (and other apps that take classic file drops) works now.
- The Screenshot key does nothing while a game is in front (like macros and actions).

## 1.0.8 (2026-10-12)
- Controller: the "Restart Steam to apply" button is gone - restarting Steam could make Steam reload a game's controller layout from its cloud copy and replace your own (stick dead zones changed). The app never restarts Steam.
- Controller settings: the rows that Steam ignores while it runs (stick dead zones, anti-drift, gyro noise filter, rumble, light colour and brightness) are hidden until they can apply live. Button sounds stays. Per-game button, stick and trigger changes work as before (live).

## 1.0.7 (2026-10-11)
- Key, mouse and controller button windows: the Press / Release lists also offer the sound packs (built-in, your own, downloaded), not only your own file.
- "Make a pack from one sound…" is now simply "Make a pack…".
- Noise: the waves are gone - the Noise tab no longer redraws while noise plays (0 CPU); the quiet "time listened" line stays.

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
