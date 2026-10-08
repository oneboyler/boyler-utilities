//! The Performance tab (menu-v22 page `pc`), Order 021: the live tiles (CPU, GPU, RAM, Disk, Network, each with the last
//! 40 s as a calm line), "Your PC" (8 facts, Copy all) and the process list (sort, search, Live order, Windows, End on
//! hover, the right-click menu). Live ONLY while the page is open (bu-perf's sampler + a process refresh once a second on
//! one worker thread, stopped on close). Benchmarks and the CPU temperature are PARKED (Order 021): the CPU tile keeps
//! the drawing's "— °C" pill (bu-perf: unavailable without a sensor driver).

mod svc;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use bu_perf::processes::{self, EndRule, ProcessMonitor, ProcessRow, SortBy};
use bu_perf::specs::PcSpecs;
use bu_perf::{LiveReading, Priority};
use skia_safe as sk;
use taffy::style::{AlignItems, JustifyContent};

use crate::anim::{Bezier, EASE};
use crate::gfx::{sh, Font, Rgba};
use crate::pages::{Env, Page};
use crate::png::Pixels;
use crate::ui::cx::{Cx, Ev};
use crate::ui::el::{idx, key, lh, sub, Cursor, El, Key, RADIUS_PILL};
use crate::ui::pieces::button::Kind;
use crate::ui::pieces::listrow::{tile, Tile};
use crate::ui::pieces::mbtn::{self, Mb};
use crate::ui::pieces::mitems::{self, It, Place, Right, Row};
use crate::ui::pieces::{self, badge, group, rowbits, search, toast};
use crate::ui::{cmix, ACC, AMBER, CTL, CTL_H, FG, FG2, FG3, GREEN, GRP, HAIR, HOV, ICO_ON, POP, RED, SEL, TRK, VZ2, WHITE};

use svc::{Cmd, Msg, Snap, Worker};

const K_COPY: Key = key("pc.copy");
const K_WPS: Key = key("pc.wps");
const K_LIVE: Key = key("pc.live");
const K_SEARCH: Key = key("pc.search");
const K_SORT: Key = key("pc.sort");
const K_ROW: Key = key("pc.row");
const K_END: Key = key("pc.end");
const K_ASK: Key = key("pc.ask");
const K_MENU: Key = key("pc.menu");
const K_TOAST: Key = key("pc.toast");
const K_TMP: Key = key("pc.tmp");
const K_PCN: Key = key("pc.pcn");
const K_LOCK: Key = key("pc.lock");
const K_SPEC: Key = key("pc.spec");
const COLS: [(&str, SortBy); 4] = [("Name", SortBy::Name), ("CPU", SortBy::Cpu), ("RAM", SortBy::Ram), ("GPU", SortBy::Gpu)];
const MB: f64 = 1024.0 * 1024.0;
const GB: f64 = 1024.0 * 1024.0 * 1024.0;
const EASE_OUT: Bezier = Bezier::new(0.0, 0.0, 0.58, 1.0);

/// The drawing's process tiles (`PROCS[].g`, `bg`) by exe, for test copies (a real copy shows each exe's own icon).
fn proc_look(exe: &str) -> (&'static str, u32, u32) {
    match exe.to_lowercase().as_str() {
        "valorant-win64-shipping.exe" | "valorant.exe" => ("pad", 0xff7a76, 0xd83f4c),
        "chrome.exe" => ("globe", 0xffd35a, 0xe6493b),
        "discord.exe" => ("chat", 0x8f95ff, 0x5a5fe0),
        "obs64.exe" => ("rec", 0x7a808c, 0x3a3e47),
        "spotify.exe" => ("note", 0x46d989, 0x1c9a5a),
        "steam.exe" => ("pad", 0x6f8fb8, 0x2b3f5c),
        "explorer.exe" => ("fold", 0xffd35a, 0xe8a33a),
        "nvcontainer.exe" => ("chip", 0x9be15d, 0x4a9a1c),
        "vgtray.exe" | "vgc.exe" => ("shd16", 0xff7a76, 0xd83f4c),
        "wootility.exe" => ("kb16", 0xffb86b, 0xe0661c),
        "dwm.exe" => ("win16", 0x7fb2ff, 0x3b6fd6),
        "msmpeng.exe" => ("shd16", 0x7fb2ff, 0x3b6fd6),
        "lsass.exe" => ("shd16", 0xa2abbd, 0x6c7487),
        "winlogon.exe" | "system" => ("win16", 0xa2abbd, 0x6c7487),
        _ => ("cog16", 0xa2abbd, 0x6c7487),
    }
}

/// "Ryzen 7 7800X3D" from "AMD Ryzen 7 7800X3D 8-Core Processor" (the tile's short name; the full one is its title).
fn short_cpu(name: &str) -> String {
    let mut s = name.replace("(R)", "").replace("(TM)", "").replace(" CPU", "");
    for p in ["AMD ", "Intel Core ", "Intel "] {
        if let Some(r) = s.strip_prefix(p) {
            s = r.to_string();
        }
    }
    let s = match s.find(" with ") {
        Some(i) => s[..i].to_string(),
        None => s,
    };
    let words: Vec<&str> = s.split_whitespace().filter(|w| !w.ends_with("-Core") && *w != "Processor" && !w.starts_with('@')).collect();
    words.join(" ")
}

/// The drawing's `GBf` for the free space ("612 GB", "1.82 TB", "12.4 GB").
fn gbf(bytes: u64) -> String {
    let v = bytes as f64 / GB;
    if v >= 1000.0 {
        format!("{:.2} TB", v / 1024.0)
    } else if v >= 100.0 {
        format!("{:.0} GB", v)
    } else if v >= 1.0 {
        format!("{v:.1} GB")
    } else {
        format!("{:.0} MB", (v * 1024.0).max(1.0))
    }
}

/// Copies text to the clipboard through Windows' own clip.exe (UTF-16 with its BOM). TEMP until a shared clipboard helper
/// (PIECES_WANTED); only on the user's click.
fn to_clipboard(text: &str) -> bool {
    use std::io::Write;
    let Ok(mut c) = std::process::Command::new("clip.exe").stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::null()).spawn() else {
        return false;
    };
    let mut bytes = vec![0xFF, 0xFE];
    for u in text.encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    let ok = c.stdin.take().map(|mut s| s.write_all(&bytes).is_ok()).unwrap_or(false);
    ok && c.wait().map(|s| s.success()).unwrap_or(false)
}

/// The list's sort: column + descending (the default: CPU, highest first).
#[derive(Clone, Copy)]
struct Sort(SortBy, bool);

impl Default for Sort {
    fn default() -> Self {
        Sort(SortBy::Cpu, true)
    }
}

enum Pop {
    /// "End <name>?" by its End button / menu: row key, tree?, at (x, y)
    Ask(String, bool, f32, f32),
    /// the right-click menu (`prMenu`): row key, at the pointer; `prio` = it turned into the priority list
    Menu(String, f32, f32, bool),
}

#[derive(Default)]
pub struct Performance {
    /// the app's store: the last snapshot is kept there when the tab closes (shown at once next time, "Your PC" not read again)
    keep: crate::keep::Keep,
    test: bool,
    real_read: bool,
    snap: Snap,
    worker: Option<Worker>,
    /// test copies: the fake + its monitor (synchronous, frozen at the drawing's values)
    fake: Option<(bu_perf::FakeOs, ProcessMonitor, HashMap<u32, f64>)>,
    show_windows: bool,
    /// "Live order": ON = the list re-sorts on every snapshot; OFF (the default) = the rows keep `order` and only a click on a
    /// sort header re-sorts (the numbers still change in place)
    live: bool,
    /// the keys of the rows in the order they are shown while Live order is off
    order: Vec<String>,
    query: String,
    sort: Sort,
    pop: Option<Pop>,
    toast: Option<(String, f64)>,
    /// "Copy all" turned into "Copied" at
    copied: Option<f64>,
    /// Order 047: Copy all's clipboard write runs on its own thread (clip.exe is started and waited for: 50-150 ms) - its
    /// answer (copied?) arrives here, `tick` takes it
    copying: Option<Arc<std::sync::Mutex<Option<bool>>>>,
    /// the clipboard write (None = Windows' clip.exe; a test hands a slow stand-in)
    clip: Option<fn(&str) -> bool>,
    pressed: (f32, f32, f32, f32),
    /// rows being ended (key, since): they fold away (height 36 -> 0, 220 ms)
    ending: Vec<(String, f64)>,
    /// what Copy all / the menu did in a test copy (nothing is opened or copied there)
    pub log: Vec<String>,
}

impl Performance {
    fn rows(&self) -> Vec<ProcessRow> {
        let mut r = self.snap.rows.clone();
        if self.live {
            processes::sort(&mut r, self.sort.0, self.sort.1);
        } else {
            // (a map, not a search per row: this runs on every build of the page)
            let at: HashMap<&str, usize> = self.order.iter().enumerate().map(|(i, k)| (k.as_str(), i)).collect();
            r.sort_by_key(|r| at.get(r.key.as_str()).copied().unwrap_or(usize::MAX));
        }
        let q =self.query.trim().to_lowercase();
        r.into_iter().filter(|p| q.is_empty() || p.name.to_lowercase().contains(&q)).collect()
    }

    /// A new snapshot: Live order on = the shown order follows the sort; off = ended rows drop out, new ones go at the end
    /// (in sort order), nothing else moves.
    fn on_snap(&mut self) {
        if self.live {
            return self.resort();
        }
        let mut r = self.snap.rows.clone();
        processes::sort(&mut r, self.sort.0, self.sort.1);
        self.order.retain(|k| r.iter().any(|p| p.key == *k));
        let new: Vec<String> = r.into_iter().map(|p| p.key).filter(|k| !self.order.contains(k)).collect();
        self.order.extend(new);
    }

    /// The shown order = the current sort, once.
    fn resort(&mut self) {
        let mut r = self.snap.rows.clone();
        processes::sort(&mut r, self.sort.0, self.sort.1);
        self.order = r.into_iter().map(|p| p.key).collect();
    }

    fn row(&self, key: &str) -> Option<ProcessRow> {
        self.snap.rows.iter().find(|r| r.key == key).cloned()
    }

    fn show_toast(&mut self, t: impl Into<String>, now: f64) {
        self.toast = Some((t.into(), now));
    }

    /// Copy all's answer: "Copied" + its toast, or Windows' refusal.
    fn copy_done(&mut self, ok: bool, now: f64) {
        if ok {
            self.copied = Some(now);
            self.show_toast("Your PC copied · paste it anywhere", now);
        } else {
            self.show_toast("Windows didn’t take the copy", now);
        }
    }

    fn refresh_fake(&mut self) {
        if let Some((os, mon, gpu)) = &mut self.fake {
            self.snap.rows = svc::sample_rows(os, mon, gpu, self.show_windows);
            self.on_snap();
        }
    }

    /// End (or End process tree) a row the user confirmed / that ends straight away.
    fn end(&mut self, key: &str, tree: bool, now: f64) {
        let Some(r) = self.row(key) else { return };
        if self.real_read {
            self.show_toast("A read-only test copy changes nothing", now);
            return;
        }
        self.ending.push((key.to_string(), now));
        if let Some((os, mon, _)) = &mut self.fake {
            let rep = if tree { mon.end_tree(os, &r, true) } else { mon.end_task(os, &r, true) };
            let t = match rep {
                Ok(rep) => rep.toast(&r.name),
                Err(e) => e.to_string(),
            };
            self.show_toast(t, now);
            return;
        }
        if let Some(w) = &self.worker {
            w.send(Cmd::End { key: key.to_string(), tree });
        }
    }

    /// The row's End button / the menu's End task: a plain app ends at once, anything else asks first (by the button).
    fn end_pressed(&mut self, key: &str, tree: bool, at: (f32, f32), now: f64) {
        let Some(r) = self.row(key) else { return };
        match r.end_rule {
            EndRule::Locked => {}
            EndRule::Instant => self.end(key, tree, now),
            EndRule::AskFirst { .. } => self.pop = Some(Pop::Ask(key.to_string(), tree, at.0, at.1)),
        }
    }

    /// The right-click menu of a row at the pointer (frame: `Ev::Context` - TEMP until Lane K's right-click, A_021_01).
    pub fn context(&mut self, key: &str, x: f32, y: f32) {
        self.pop = Some(Pop::Menu(key.to_string(), x, y, false));
    }

    // ------------------------------------------------------------------ building

    /// `.tmp{display:inline-flex;align-items:center;gap:5px;height:18px;padding:0 7px 0 6px;border-radius:9px;background:var(--ctl);
    /// font-size:11px;font-weight:600;color:var(--fg);font-variant-numeric:tabular-nums}` `i{5 x 5, 50%, var(--green)}`
    /// `.warm i{amber}` `.hot i{red}` `.na{color:var(--fg3)}` `.na i{background:var(--fg3)}`
    fn tmp(c: Option<f64>) -> El {
        let (text, dot, fg) = match c {
            Some(c) => (format!("{} °C", c.round()), if c >= 85.0 { RED() } else if c >= 75.0 { AMBER() } else { GREEN() }, FG()),
            None => ("— °C".to_string(), FG3(), FG3()),
        };
        El::row()
            .center()
            .gap(5.0)
            .h(18.0)
            .none()
            .pad(0.0, 7.0, 0.0, 6.0)
            .radius(9.0)
            .bg(CTL())
            .child(El::block().size(5.0, 5.0).radius(RADIUS_PILL).bg(dot))
            .child(El::text(text, Font::new(11.0, 600).tnum(), fg, lh(11.0, 1.35)))
    }

    /// `.pcv{font:600 20px/26px "Segoe UI Variable Display";letter-spacing:-.01em;tabular-nums}` + `small{font-size:13px;
    /// font-weight:600;color:var(--fg2);margin-left:2px}` on one baseline: the small's 26 px line box sits 3 px lower
    /// (ascent 22 vs 14, half-leading -1 vs 4 - measured in the drawing: the value box is 29 px, the unit's text 7 px below
    /// the value's top).
    fn pcv(big: &str, unit: &str) -> El {
        El::row()
            .items(AlignItems::FLEX_START)
            .none()
            .h(29.0)
            .child(El::text(big, Font::display(20.0, 600).ls(-200).tnum(), FG(), 26.0).none())
            // the unit inherits `.pcv`'s family (Segoe UI Variable Display) and letter-spacing as a length (-.01em x 20 px = -0.2 px per
            // letter): its widths then equal Chromium's (% 10.547, GB 16.516, Mb/s 30.484, measured)
            .child(El::text(unit, Font::display(13.0, 600).ls(-200).tnum(), FG2(), 26.0).none().margin(3.0, 0.0, 0.0, 2.0))
    }

    /// `.pcx{font-size:11.5px;color:var(--fg2);tabular-nums}` `b{font-weight:600;color:var(--fg)}`: runs of (text, bold)
    /// and an optional pill (the line's text sits .5 px below the pill's top, as Chromium lays the inline-flex pill out).
    fn pcx(runs: &[(&str, bool)], pill: Option<(El, bool)>) -> (El, bool) {
        let has_pill = pill.is_some();
        (Self::pcx_el(runs, pill), has_pill)
    }

    fn pcx_el(runs: &[(&str, bool)], pill: Option<(El, bool)>) -> El {
        let f = Font::new(11.5, 400).tnum();
        let fb = Font::new(11.5, 600).tnum();
        let text = |s: &str, b: bool, top: f32| El::text(s, if b { fb } else { f }, if b { FG() } else { FG2() }, lh(11.5, 1.35)).none().margin(top, 0.0, 0.0, 0.0);
        let top = if pill.is_some() { 0.5 } else { 0.0 };
        let mut r = El::row().items(AlignItems::FLEX_START).min_w(0.0).clip();
        if let Some((p, true)) = &pill {
            r = r.child(p.clone());
        }
        for (s, b) in runs {
            r = r.child(text(s, *b, top));
        }
        if let Some((p, false)) = pill {
            r = r.child(p);
        }
        r
    }

    /// One `.ptl` tile: `.ptl{grid-column:span 2;padding:10px 12px 8px;border-radius:10px;background:var(--grp);
    /// box-shadow:inset 0 0 0 .5px var(--hair);overflow:hidden}` (`.g4` span 4) = `.pch` (label + name), `.pcm` (value +
    /// extra), `.pcs` (30 px line).
    fn ptl(label: &str, name: &str, full: &str, span: u16, value: El, (extra, extra_pill): (El, bool), series: Vec<(Vec<f64>, Rgba, f32)>, max: f64) -> El {
        // `n=h('span',{class:'pcn',text:name,title:name})` (the CPU's: `PT.cpu.n.title='AMD Ryzen 7 7800X3D'`, the full name)
        let mut pcn = El::text(name, Font::new(11.0, 400), FG3(), lh(11.0, 1.35)).ellipsis();
        if !full.is_empty() {
            pcn = pcn.key(sub(K_PCN, label)).title(full);
        }
        let pch = El::row()
            .center()
            .gap(8.0)
            .child(El::text(label, Font::new(11.0, 600).ls(220), FG2(), lh(11.0, 1.35)).none())
            .child(pcn);
        // `.pcm{display:flex;align-items:center;gap:8px;margin-top:3px}`: Blink centres the extra line in LayoutUnits, rounded
        // down to 1/64 px ((29 - 15.516) / 2 = 6.734, not 6.742); with the pill the line is 18 px ((29 - 18) / 2 = 5.5)
        let top = if extra_pill { 5.5 } else { 431.0 / 64.0 };
        let pcm = El::row().items(AlignItems::FLEX_START).gap(8.0).margin(3.0, 0.0, 0.0, 0.0).min_w(0.0).child(value).child(extra.margin(top, 0.0, 0.0, 0.0));
        let line = El::paint(move |g, (x, y, w, h)| spark(g, (x, y, w, h), &series, max)).h(30.0).margin(6.0, 0.0, 0.0, 0.0).live();
        El::col()
            .min_w(0.0)
            .pad(10.0, 12.0, 8.0, 12.0)
            .radius(10.0)
            .bg(GRP())
            .inset(&[sh(0.0, 0.0, 0.0, 0.5, HAIR())])
            .clip()
            .style(move |s| {
                s.grid_column = taffy::geometry::Line { start: taffy::style::GridPlacement::Span(span), end: taffy::style::GridPlacement::Auto };
            })
            .child(pch)
            .child(pcm)
            .child(line)
    }

    fn tiles(&self) -> El {
        let l = self.snap.latest.clone().unwrap_or_default();
        let specs = self.snap.specs.clone().unwrap_or_default();
        let hist = &self.snap.history;
        let ser = |f: &dyn Fn(&LiveReading) -> f64| hist.iter().map(f).collect::<Vec<f64>>();
        let have = self.snap.latest.is_some();
        let num = |s: String| if have { s } else { "—".to_string() };
        // CPU: usage + clock; temperature "— °C" (PARKED: needs a sensor driver)
        let cpu = Self::ptl(
            "CPU",
            &short_cpu(&specs.cpu.name),
            &specs.cpu.name,
            2,
            Self::pcv(&num(format!("{:.0}", l.cpu_usage_pct)), "%"),
            // `h('span',{class:'tmp na','data-tip':'Needs a hardware sensor driver — see research'},..)`
            Self::pcx(
                &[(&num(format!("{:.2}", l.cpu_mhz / 1000.0)), true), (" GHz \u{a0}·\u{a0} ", false)],
                Some((Self::tmp(None).key(K_TMP).tip("Needs a hardware sensor driver — see research"), false)),
            ),
            vec![(ser(&|r| r.cpu_usage_pct), ACC(), 1.0)],
            100.0,
        );
        // GPU: the card (not the integrated one)
        let main = |r: &LiveReading| bu_perf::live::main_gpu(r).cloned();
        let g = main(&l).unwrap_or_default();
        let gname = if g.name.is_empty() { specs.gpus.iter().find(|x| !x.integrated).map(|x| x.name.clone()).unwrap_or_default() } else { g.name.clone() };
        let gb = |b: u64| b as f64 / GB;
        let mut gruns: Vec<(String, bool)> = vec![(" \u{a0}VRAM ".into(), false), (format!("{:.1}", gb(g.vram_used_bytes)), true), (format!(" / {:.0} GB", gb(g.vram_total_bytes)), false)];
        if let Some(f) = g.fan_pct {
            gruns.push((" \u{a0}·\u{a0} Fan ".into(), false));
            gruns.push((f.to_string(), true));
            gruns.push((" %".into(), false));
        } else if let Some(rpm) = g.fan_rpm.filter(|&r| r > 0) {
            gruns.push((" \u{a0}·\u{a0} Fan ".into(), false));
            gruns.push((rpm.to_string(), true));
            gruns.push((" rpm".into(), false));
        }
        let gr: Vec<(&str, bool)> = gruns.iter().map(|(s, b)| (s.as_str(), *b)).collect();
        let gpu = Self::ptl(
            "GPU",
            &gname,
            &gname,
            4,
            Self::pcv(&num(format!("{:.0}", g.usage_pct)), "%"),
            Self::pcx(&gr, Some((Self::tmp(g.temperature_c.or(Some(f64::NAN)).filter(|c| !c.is_nan())), true))),
            vec![(hist.iter().map(|r| main(r).map(|g| g.usage_pct).unwrap_or(0.0)).collect(), ACC(), 1.0)],
            100.0,
        );
        // RAM: "12.6 GB", "of 32 · 39 %"
        let total = l.ram_total_bytes as f64 / GB;
        let ram_name = match (&specs.ram.kind, specs.ram.total_bytes) {
            (Some(k), t) if t > 0 => format!("{:.0} GB {k}", t as f64 / GB),
            _ => String::new(),
        };
        let ram = Self::ptl(
            "RAM",
            &ram_name,
            &ram_name,
            2,
            Self::pcv(&num(format!("{:.1}", l.ram_used_bytes as f64 / GB)), "GB"),
            Self::pcx(&[(&num(format!("of {:.0} · {:.0} %", total, if total > 0.0 { l.ram_used_bytes as f64 / GB / total * 100.0 } else { 0.0 })), false)], None),
            vec![(ser(&|r| r.ram_used_bytes as f64 / GB), ACC(), 1.0)],
            total.max(1.0),
        );
        // Disk: the Windows drive ("C: Samsung 990 PRO"), active %, MB/s · free
        let letter = std::env::var("SystemDrive").ok().and_then(|d| d.chars().next()).unwrap_or('C');
        let d = bu_perf::live::main_disk(&l, letter).cloned().unwrap_or_default();
        let dnum: Option<usize> = d.instance.split_whitespace().next().and_then(|n| n.parse().ok());
        let dmodel = dnum.and_then(|n| specs.drives.get(n)).map(|x| x.model.clone()).unwrap_or_default();
        let free = l.system_free_bytes.map(|f| format!(" · {} free", gbf(f))).unwrap_or_default();
        let disk_name = format!("{letter}: {dmodel}");
        let disk = Self::ptl(
            "Disk",
            disk_name.trim(),
            disk_name.trim(),
            2,
            Self::pcv(&num(format!("{:.0}", d.active_pct)), "%"),
            Self::pcx(&[(&num(format!("{:.0} MB/s{free}", d.bytes_per_sec / 1e6)), false)], None),
            vec![(hist.iter().map(|r| bu_perf::live::main_disk(r, letter).map(|d| d.active_pct).unwrap_or(0.0)).collect(), ACC(), 1.0)],
            100.0,
        );
        // Network: "Ethernet · 1 Gbps", ↓ in the accent, ↑ in teal
        let mut nets: Vec<&bu_perf::specs::NetSpec> = specs.nets.iter().filter(|n| n.connected).collect();
        nets.sort_by_key(|n| n.wireless);
        let nname = nets
            .first()
            .map(|n| {
                let kind = if n.wireless { "Wi-Fi" } else { "Ethernet" };
                n.speed_bps.map(|b| format!("{kind} · {}", bu_perf::specs::link_speed(b))).unwrap_or_else(|| kind.to_string())
            })
            .unwrap_or_default();
        let net = Self::ptl(
            "Network",
            &nname,
            &nname,
            2,
            Self::pcv(&num(format!("↓ {:.1}", l.net_down_bps / 1e6)), "Mb/s"),
            Self::pcx(&[(&num(format!("↑ {:.1} Mb/s", l.net_up_bps / 1e6)), false)], None),
            vec![(ser(&|r| r.net_down_bps / 1e6), ACC(), 1.0), (ser(&|r| r.net_up_bps / 1e6), VZ2(), 0.85)],
            40.0,
        );
        // `.pcg{display:grid;grid-template-columns:repeat(6,minmax(0,1fr));gap:8px;margin-top:2px}`
        El::grid().cols(6).gap(8.0).margin(2.0, 0.0, 0.0, 0.0).children([cpu, gpu, ram, disk, net])
    }

    /// "Your PC": `.spg{display:grid;grid-template-columns:repeat(2,minmax(0,1fr))}` of `.spcell{display:flex;flex-direction:column;
    /// gap:1px;padding:9px 12px 10px}` with the lines that meet (v21): a row line 12 px in from the group's edges, the middle
    /// line unbroken from the first row to the last, its two ends inset 10 px.
    fn your_pc(&self, cx: &mut Cx) -> El {
        let mut specs = self.snap.specs.clone().unwrap_or_default();
        // the Windows drive first (the drawing: "Samsung 990 PRO · 2 TB NVMe" = C:), by its disk number in the live reading
        let letter = std::env::var("SystemDrive").ok().and_then(|d| d.chars().next()).unwrap_or('C');
        if let Some(n) = self.snap.latest.as_ref().and_then(|l| bu_perf::live::main_disk(l, letter)).and_then(|d| d.instance.split_whitespace().next()?.parse::<usize>().ok()) {
            if n < specs.drives.len() {
                let d = specs.drives.remove(n);
                specs.drives.insert(0, d);
            }
        }
        let cells = if self.snap.specs.is_some() { specs.cells() } else { Vec::new() };
        let n = cells.len();
        let mut kids = Vec::new();
        for (i, c) in cells.iter().enumerate() {
            let even = i % 2 == 1;
            // `h('b',{text:x.v,title:x.v}),h('small',{text:x.s,title:x.s})`
            let titled = |e: El, part: &str, t: &str| if t.is_empty() { e } else { e.key(sub(idx(K_SPEC, i), part)).title(t) };
            let mut cell = El::col()
                .gap(1.0)
                .min_w(0.0)
                .pad(9.0, 12.0, 10.0, 12.0)
                .child(El::text(c.label, Font::new(11.0, 600).ls(220), FG2(), lh(11.0, 1.35)))
                .child(titled(El::text(c.value.clone(), Font::new(12.5, 600), FG(), 17.0).ellipsis(), "v", &c.value))
                .child(titled(El::text(c.quiet.clone(), Font::new(11.0, 400), FG3(), 14.0).ellipsis(), "s", &c.quiet));
            if i >= 2 {
                // `.spcell::before{left:12px;right:0;top:0;height:1px}` `:nth-child(2n)::before{left:0;right:12px}`
                cell = cell.child(if even { El::block().abs(0.0, 0.0, 12.0, f32::NAN) } else { El::block().abs(12.0, 0.0, 0.0, f32::NAN) }.h(1.0).bg(HAIR()).no_hit());
            }
            if even {
                // `.spcell:nth-child(2n)::after{left:0;top:0;bottom:0;width:1px}` (2nd: top 10; the last even one: bottom 10)
                let top = if i == 1 { 10.0 } else { 0.0 };
                let bottom = if i + 1 == n { 10.0 } else { 0.0 };
                cell = cell.child(El::block().abs(0.0, top, f32::NAN, bottom).w(1.0).bg(HAIR()).no_hit());
            }
            kids.push(cell);
        }
        let rim = if crate::ui::is_light() { crate::ui::pieces::group::glass().1 } else { vec![sh(0.0, 0.0, 0.0, 1.0, crate::ui::GRP_RIM()), sh(0.0, 1.0, 0.0, 0.0, crate::ui::GRP_TOP())] };
        let grid = El::grid().cols(2).bg(GRP()).radius(10.0).inset(&rim).children(kids);
        // the header's small button `.mbtn` ("Copy all" -> "Copied" for 1.6 s)
        let done = self.copied.is_some_and(|t| cx.now - t < mbtn::DONE_MS);
        // Order 047: "Copied" goes back to "Copy all" at a known moment - built again then, no frames until it
        if let Some(t) = self.copied.filter(|_| done) {
            cx.wake_at(t + mbtn::DONE_MS);
        }
        let btn = mbtn::mbtn(cx, K_COPY, if done { Mb::Done("Copied") } else { Mb::Text("Copy all") }, false);
        let gh = mbtn::gh_with("Your PC", vec![], vec![btn]);
        El::block().child(gh).child(grid)
    }

    fn processes(&self, cx: &mut Cx) -> El {
        let rows = self.rows();
        // header: "Processes" + count, then (right) the Windows processes switch + the small search
        let shown = self.snap.rows.len();
        let live = rowbits::wps(cx, K_LIVE, "Live order", self.live, rowbits::WpsAt::Header);
        let wps = rowbits::wps(cx, K_WPS, "Windows", self.show_windows, rowbits::WpsAt::Header);
        let gh = group::gh("Processes")
            .child(El::text(shown.to_string(), Font::new(11.0, 400), FG3(), lh(11.0, 1.35)))
            .child(El::row().center().gap(10.0).ml_auto().child(live).child(wps).child(search::search(cx, K_SEARCH, &self.query, "Search processes", true)));
        // `.prh,.prr{display:grid;grid-template-columns:minmax(0,1fr) 62px 74px 54px (+ the End column 44px);align-items:center;column-gap:6px;padding:0 12px 0 10px}`
        let grid = |e: El| {
            e.style(|s| {
                s.grid_template_columns = vec![
                    taffy::style_helpers::minmax(taffy::style_helpers::length(0.0), taffy::style_helpers::fr(1.0)),
                    taffy::style_helpers::length(62.0),
                    taffy::style_helpers::length(74.0),
                    taffy::style_helpers::length(54.0),
                    // the End button's own column (Order 043: it used to sit over the GPU number)
                    taffy::style_helpers::length(44.0),
                ];
                s.gap.width = taffy::style::LengthPercentage::length(6.0);
                s.align_items = Some(AlignItems::CENTER);
            })
            .pad(0.0, 12.0, 0.0, 10.0)
        };
        // the sort buttons: `#sw .prh button{display:flex;align-items:center;justify-content:flex-end;gap:4px;height:22px;margin:0 -5px;
        // padding:0 5px;border-radius:5px;font-size:11px;font-weight:600;color:var(--fg3)}` `:hover{background:var(--hov);color:var(--fg2)}`
        // `.on{color:var(--fg)}` `.nm{justify-content:flex-start;margin-left:24px}` `i{8 x 5;opacity:0}` `.on i{opacity:1}` `.up i{rotate(180deg)}`
        let mut head = grid(El::grid()).h(30.0).inset(&[sh(0.0, -1.0, 0.0, 0.0, HAIR())]);
        for (i, (label, by)) in COLS.iter().enumerate() {
            let k = idx(K_SORT, i);
            let on = self.sort.0 == *by;
            let hv = cx.hover_t(k, 120.0, EASE);
            let op = cx.tr(k, 1, if on { 1.0 } else { 0.0 }, 120.0, EASE);
            let up = cx.tr(k, 2, if on && !self.sort.1 { 1.0 } else { 0.0 }, 200.0, EASE);
            let col = if on { FG() } else { cmix(FG3(), FG2(), hv) };
            let b = El::row()
                .center()
                .gap(4.0)
                .h(22.0)
                .justify(if i == 0 { JustifyContent::FLEX_START } else { JustifyContent::FLEX_END })
                .margin(0.0, -5.0, 0.0, if i == 0 { 24.0 } else { -5.0 })
                .pad(0.0, 5.0, 0.0, 5.0)
                .radius(5.0)
                .bg(HOV().mul_a(hv))
                .on_click(k)
                .cursor(Cursor::Hand)
                .child(El::text(*label, pieces::btn_font(11.0, 600), col, lh(11.0, 1.35)).none())
                .child(El::icon("cd", 8.0, 1.4, col).h(5.0).opacity(op).rotate(180.0 * up));
            head = head.child(b);
        }
        let mut list = El::block();
        let mut first = true;
        for (n, r) in rows.iter().enumerate() {
            let k = idx(K_ROW, n);
            let hover = cx.hovered(k) || matches!(&self.pop, Some(Pop::Menu(key, ..)) if *key == r.key);
            let prot = r.windows_own || r.protected;
            let look = proc_look(&r.exe);
            let t = match (self.test, r.path.as_ref().and_then(|p| self.snap.icons.get(p))) {
                (false, Some(px)) => Tile::Icon(px.clone()),
                _ => Tile::Glyph { glyph: look.0, a: Rgba::hex(look.1), b: Rgba::hex(look.2) },
            };
            // `.prr.prot .ait{filter:grayscale(1);opacity:.5}`
            let t = match (prot, t) {
                (true, Tile::Glyph { glyph, a, b }) => Tile::Glyph { glyph, a: a.gray(), b: b.gray() },
                (_, t) => t,
            };
            let tile_el = tile(&t, 20.0).opacity(if prot { 0.5 } else { 1.0 });
            // `.prn{display:flex;align-items:center;gap:9px}` `.prn>span{font-size:12.5px;ellipsis}` `em{color:var(--fg3);margin-left:4px;font-size:11.5px}`
            let name_col = if prot { FG3() } else { FG() };
            let mut nm = El::row().items(AlignItems::BASELINE).min_w(0.0).child(El::text(r.name.clone(), Font::new(12.5, 400), name_col, lh(12.5, 1.35)).ellipsis());
            if r.pids.len() > 1 {
                nm = nm.child(El::text(format!("({})", r.pids.len()), Font::new(11.5, 400), FG3(), lh(12.5, 1.35)).none().margin(0.0, 0.0, 0.0, 4.0));
            }
            let mut prn = El::row().center().gap(9.0).min_w(0.0).child(tile_el).child(nm);
            if prot {
                // `.prk{display:grid;place-items:center;width:14px;height:14px;color:var(--fg3)}` `svg{11px;stroke-width:1.3}`
                // `'data-tip':'Part of Windows · locked'`
                prn = prn.child(
                    El::block().size(14.0, 14.0).none().place_center().key(idx(K_LOCK, n)).tip("Part of Windows · locked").child(El::icon("lock", 11.0, 1.3, FG3())),
                );
            }
            if r.priority != Priority::Normal {
                // `.prp{height:16px;padding:0 6px;border-radius:5px;background:var(--sel);color:var(--ico-on);font-size:10px;font-weight:600}`
                prn = prn.child(El::row().h(16.0).none().pad(0.0, 6.0, 0.0, 6.0).radius(5.0).bg(SEL()).child(El::text(r.priority.name(), Font::new(10.0, 600), ICO_ON(), 16.0)));
            }
            let (bc, br, bg) = r.bright();
            // `.prc{text-align:right;font-size:12px;color:var(--fg2);tabular-nums}` `.hi{color:var(--fg)}` `.prot .prc{color:var(--fg3)}`
            let cell = |s: String, hi: bool| {
                El::text(s, Font::new(12.0, 400).tnum(), if prot { FG3() } else if hi { FG() } else { FG2() }, lh(12.0, 1.35)).align(crate::gfx::Align::Right)
            };
            let mut row = grid(El::grid())
                .h(36.0)
                .bg(HOV().mul_a(if hover { 1.0 } else { 0.0 }))
                .key(k)
                .child(prn)
                .child(cell(processes::format_pct(r.cpu_pct), bc))
                .child(cell(processes::format_ram(r.ram_bytes), br))
                .child(cell(processes::format_pct(r.gpu_pct), bg));
            if !first {
                row = row.child(El::block().abs(10.0, 0.0, 0.0, f32::NAN).h(1.0).bg(HAIR()).no_hit());
            }
            first = false;
            if !prot {
                // `#sw .pend{height:22px;padding:0 11px;border-radius:6px;(drawing: position:absolute;right:8px - now the row's 5th
                // grid column, its right edge 8 px in, so no number is under it)
                // background:var(--pop);box-shadow:inset 0 0 0 .5px var(--hair),0 1px 3px rgba(0,0,0,.14);font-size:11.5px;font-weight:600;
                // color:var(--red);opacity:0;transform:translateX(4px)}` row hover: opacity 1, translateX(0) (.12 s / .16 s)
                // `:hover{background:rgba(255,69,58,.16)}`
                let ek = idx(K_END, n);
                let op = cx.tr(ek, 1, if hover { 1.0 } else { 0.0 }, 120.0, EASE);
                let dx = cx.tr(ek, 2, if hover { 0.0 } else { 4.0 }, 160.0, EASE);
                let eh = cx.hover_t(ek, 120.0, EASE);
                let mut end = El::row()
                    .center()
                    .style(|s| s.justify_self = Some(taffy::style::AlignSelf::FLEX_END))
                    .margin(0.0, -4.0, 0.0, 0.0)
                    .h(22.0)
                    .pad(0.0, 11.0, 0.0, 11.0)
                    .radius(6.0)
                    .bg(cmix(POP(), Rgba::rgba(255, 69, 58, 0.16), eh))
                    .inset(&[sh(0.0, 0.0, 0.0, 0.5, HAIR())])
                    .shadow(&[sh(0.0, 1.0, 3.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.14))])
                    .opacity(op)
                    .translate(dx, 0.0)
                    .child(El::text("End", pieces::btn_font(11.5, 600), RED(), lh(11.5, 1.35)));
                // `title:p.ask?'End task · asks first':'End task'`
                let ask = matches!(r.end_rule, EndRule::AskFirst { .. });
                end = if op > 0.01 { end.on_click(ek).cursor(Cursor::Hand).title(if ask { "End task · asks first" } else { "End task" }) } else { end.no_hit() };
                row = row.child(end);
            }
            // ending: the row folds away (height 36 -> 0, opacity 1 -> 0, 220 ms ease-out)
            if let Some((_, at)) = self.ending.iter().find(|(key, _)| *key == r.key) {
                let k2 = ((cx.now - at) / 220.0).clamp(0.0, 1.0);
                let v = EASE_OUT.ease(k2) as f32;
                // (real motion, only while it folds; Order 047: never clears what another part asked for)
                if k2 < 1.0 {
                    cx.st.busy = true;
                }
                row = row.h(36.0 * (1.0 - v)).opacity(1.0 - v).clip();
            }
            list = list.child(row);
        }
        if rows.is_empty() && !self.query.trim().is_empty() {
            // `.pnone{padding:22px 0;text-align:center;font-size:12.5px;color:var(--fg2)}`
            list = list.child(El::text(format!("No process matches “{}”", self.query.trim()), Font::new(12.5, 400), FG2(), lh(12.5, 1.35)).align(crate::gfx::Align::Center).pad(22.0, 0.0, 22.0, 0.0));
        }
        let grp = group::grp(vec![head, list]).clip();
        El::block().child(gh).child(grp)
    }
}

/// The drawing's `spark()`: per series a 1.5 px line (round joins) over the last 40 s (step = width / 39, newest at the right
/// edge), the first series with a soft fill under it (its colour at 25 % -> 0 top to bottom); a second series at .85 alpha.
fn spark(g: &crate::gfx::Gfx, (x, y, w, h): (f32, f32, f32, f32), series: &[(Vec<f64>, Rgba, f32)], max: f64) {
    for (si, (s, c, a)) in series.iter().enumerate() {
        if s.len() < 2 {
            continue;
        }
        let step = w / 39.0;
        let x0 = x + w - (s.len() as f32 - 1.0) * step;
        let py = |v: f64| y + h - 1.5 - (v.min(max) / max) as f32 * (h - 4.0);
        let mut d = String::new();
        for (i, v) in s.iter().enumerate() {
            d.push_str(&format!("{}{} {}", if i == 0 { "M" } else { "L" }, x0 + i as f32 * step, py(*v)));
        }
        let line = g.path(&d);
        if si == 0 {
            let fill = g.path(&format!("{d}L{} {}L{} {}Z", x + w, y + h, x0, y + h));
            let mut p = sk::Paint::default();
            p.set_anti_alias(true);
            p.set_shader(g.hgrad(x, y, x, y + h, &[(0.0, c.mul_a(64.0 / 255.0)), (1.0, c.mul_a(0.0))]));
            g.cv().draw_path(&fill, &p);
        }
        g.stroke_geom(&line, 1.5, c.mul_a(*a));
    }
}

impl Page for Performance {
    fn id(&self) -> &'static str {
        "pc"
    }
    fn name(&self) -> &'static str {
        "Performance"
    }
    fn icon(&self) -> &'static str {
        "gauge"
    }
    fn open(&mut self, env: &Env, _now: f64) {
        self.test = env.fake();
        self.real_read = env.real_read;
        if env.fake() {
            let (os, gpu) = svc::sample_os();
            let mut mon = ProcessMonitor::new(&PathBuf::from(r"C:\Windows"));
            self.snap.rows = svc::sample_rows(&os, &mut mon, &gpu, false);
            let r = svc::sample_reading(gpu.clone());
            self.snap.history = vec![r.clone()];
            self.snap.latest = Some(r);
            self.snap.specs = Some(svc::sample_specs());
            self.fake = Some((os, mon, gpu));
            self.on_snap();
            return;
        }
        #[cfg(windows)]
        {
            let os: Arc<dyn bu_perf::PerfOs> = if env.real_read { Arc::new(bu_perf::RealOs::read_only()) } else { Arc::new(bu_perf::RealOs::new()) };
            self.keep = env.keep.clone();
            if let Some(s) = env.keep.get::<Snap>("pc.snap") {
                self.snap = s;
                self.on_snap();
            }
            self.worker = Some(Worker::start(os, env.waker(), self.snap.specs.clone()));
        }
    }
    fn close(&mut self) {
        // dropping the worker stops the sampler + the refresh and waits for the thread: nothing runs while closed; the last
        // snapshot is kept (F2: the tab shows it at once next time while the first new one is taken)
        if self.worker.is_some() && !self.snap.rows.is_empty() {
            self.keep.put("pc.snap", std::mem::take(&mut self.snap));
        }
        *self = Performance::default();
    }
    fn ready(&self) -> bool {
        self.worker.is_none() || !self.snap.rows.is_empty() || self.snap.err.is_some()
    }
    fn tick(&mut self, now: f64) -> bool {
        let mut changed = false;
        let msgs: Vec<Msg> = self.worker.as_ref().map(|w| w.rx.try_iter().collect()).unwrap_or_default();
        for m in msgs {
            match m {
                Msg::Snap(s) => {
                    self.snap = *s;
                    self.on_snap();
                }
                Msg::Toast(t) => self.toast = Some((t, now)),
            }
            changed = true;
        }
        // Order 047: Copy all's answer from its thread (it woke the menu)
        if let Some(ok) = self.copying.as_ref().and_then(|s| s.lock().ok().and_then(|mut g| g.take())) {
            self.copying = None;
            self.copy_done(ok, now);
            changed = true;
        }
        self.ending.retain(|(k, at)| now - at < 1500.0 && self.snap.rows.iter().any(|r| r.key == *k));
        // Order 047: a toast at rest needs no frames (the toast piece wakes the menu when it fades); it goes at its end
        if self.toast.as_ref().is_some_and(|(_, t)| now - t >= toast::SHOW_MS + 300.0) {
            self.toast = None;
            changed = true;
        }
        changed
    }
    /// Order 047: the toast's end (it is dropped then); nothing else here is timed.
    fn wake_at(&self, now: f64) -> Option<f64> {
        self.toast.as_ref().map(|(_, t)| (t + toast::SHOW_MS + 300.0).max(now + 1.0))
    }
    fn build(&mut self, cx: &mut Cx) -> Vec<El> {
        vec![pieces::header(self.name(), Some(badge::live_note("Live only while this page is open"))), self.tiles(), self.your_pc(cx), self.processes(cx)]
    }
    fn event(&mut self, ev: &Ev, cx: &mut Cx) {
        let now = cx.now;
        if let Ev::Press(_, _, _, r) = ev {
            self.pressed = *r;
        }
        let k = match ev {
            // Order 045: Ctrl+F = the process search (`if(menuShown()&&curPane==='pc'&&(e.ctrlKey||e.metaKey)&&!e.altKey&&
            // e.code==='KeyF'){e.preventDefault();psIn.focus();psIn.select();return;}`, L7993)
            Ev::Key(_, 0x46) if cx.mods.ctrl && !cx.mods.alt => {
                cx.focus(Some(K_SEARCH));
                cx.used = true;
                return;
            }
            Ev::Char(k, c) if *k == K_SEARCH => {
                search::edit_char(&mut self.query, *c);
                return;
            }
            Ev::Key(k, 0x0D) if *k == K_SEARCH => {
                cx.focus(None);
                return;
            }
            Ev::Key(k, 0x1B) if *k == K_SEARCH && self.query.is_empty() => {
                cx.focus(None);
                return;
            }
            Ev::Key(k, vk) if *k == K_SEARCH => {
                search::edit_key(&mut self.query, *vk);
                return;
            }
            Ev::Click(k) => *k,
            // the right-click menu (`contextmenu` on a process row; its End button counts as the row)
            Ev::Context(k, x, y) => {
                let rows = self.rows();
                if let Some(n) = (0..rows.len()).find(|&n| idx(K_ROW, n) == *k || idx(K_END, n) == *k) {
                    self.context(&rows[n].key.clone(), *x, *y);
                }
                return;
            }
            _ => return,
        };
        if k == sub(K_SEARCH, "x") {
            self.query.clear();
            return;
        }
        if k == K_COPY {
            if self.copied.is_some_and(|t| now - t < 1600.0) {
                return;
            }
            let text = self.snap.specs.as_ref().map(PcSpecs::copy_text).unwrap_or_default();
            if (self.test || self.real_read) && self.clip.is_none() {
                self.log.push(format!("copy:{}", text.lines().count()));
                self.copy_done(true, now);
                return;
            }
            // Order 047: clip.exe is started and waited for on its own thread; "Copied" / the toast come with its answer
            if self.copying.is_some() {
                return;
            }
            let slot = Arc::new(std::sync::Mutex::new(None));
            let out = slot.clone();
            let clip = self.clip.unwrap_or(to_clipboard as fn(&str) -> bool);
            crate::offui::spawn("pc-copy", move || {
                let ok = clip(&text);
                if let Ok(mut s) = out.lock() {
                    *s = Some(ok);
                }
            });
            self.copying = Some(slot);
            return;
        }
        if k == K_LIVE {
            self.live = !self.live;
            // switching it on re-sorts now; off keeps the rows where they are
            if self.live {
                self.resort();
            }
            return;
        }
        if k == K_WPS {
            self.show_windows = !self.show_windows;
            if let Some(w) = &self.worker {
                w.send(Cmd::ShowWindows(self.show_windows));
            }
            self.refresh_fake();
            return;
        }
        if let Some(i) = (0..COLS.len()).find(|&i| idx(K_SORT, i) == k) {
            let by = COLS[i].1;
            // click a column to sort, again = the other way (Name starts A -> Z, numbers high -> low)
            self.sort = if self.sort.0 == by { Sort(by, !self.sort.1) } else { Sort(by, by != SortBy::Name) };
            self.resort();
            return;
        }
        let rows = self.rows();
        if let Some(n) = (0..rows.len()).find(|&n| idx(K_END, n) == k) {
            let (x, y, w, h) = self.pressed;
            self.end_pressed(&rows[n].key.clone(), false, (x + w, y + h), now);
            return;
        }
        match self.pop.take() {
            Some(Pop::Ask(key, tree, x, y)) => {
                if k == sub(K_ASK, "go") {
                    self.end(&key, tree, now);
                } else if k != sub(K_ASK, "no") {
                    self.pop = Some(Pop::Ask(key, tree, x, y));
                }
            }
            Some(Pop::Menu(key, x, y, false)) => {
                let Some(r) = self.row(&key) else { return };
                let prot = r.windows_own || r.protected;
                match (0..6).find(|&i| idx(K_MENU, i) == k) {
                    // row 0 = the header, 3 = the separator
                    Some(1) if !prot => self.end_pressed(&key, false, (x, y), now),
                    Some(2) if !prot => self.end_pressed(&key, true, (x, y), now),
                    Some(4) => {
                        let f = r.path.as_ref().and_then(|p| p.file_name()).map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                        if self.test || self.real_read {
                            self.log.push(format!("open:{}", r.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default()));
                            self.show_toast(format!("Opens Explorer with {f} selected"), now);
                        } else if let Some(w) = &self.worker {
                            w.send(Cmd::OpenLocation { key: key.clone() });
                        }
                    }
                    Some(5) => self.pop = Some(Pop::Menu(key, x, y, true)),
                    _ => self.pop = Some(Pop::Menu(key, x, y, false)),
                }
            }
            Some(Pop::Menu(key, x, y, true)) => {
                if let Some(i) = (0..Priority::MENU.len()).find(|&i| idx(sub(K_MENU, "p"), i + 1) == k) {
                    let p = Priority::MENU[i];
                    let Some(r) = self.row(&key) else { return };
                    if r.priority == p {
                        return;
                    }
                    if self.real_read {
                        self.show_toast("A read-only test copy changes nothing", now);
                    } else if let Some((os, mon, _)) = &mut self.fake {
                        let t = match mon.set_priority(os, &r, p) {
                            Ok(_) => processes::PriorityUndo::toast(&r.name, p),
                            Err(e) => e.to_string(),
                        };
                        self.show_toast(t, now);
                    } else if let Some(w) = &self.worker {
                        w.send(Cmd::Priority { key, p });
                    }
                } else {
                    self.pop = Some(Pop::Menu(key, x, y, true));
                }
            }
            None => {}
        }
    }
    fn popup(&mut self, cx: &mut Cx) -> Option<El> {
        let pop = match self.pop.take() {
            Some(Pop::Ask(key, tree, x, y)) => {
                let r = self.row(&key);
                let why = match r.as_ref().map(|r| &r.end_rule) {
                    Some(EndRule::AskFirst { why }) => why.clone(),
                    _ => String::new(),
                };
                let name = r.as_ref().map(|r| r.name.clone()).unwrap_or_default();
                let el = mitems::confirm(cx, K_ASK, &format!("End {name}?"), &why, "Cancel", if tree { "End tree" } else { "End task" }, Kind::Red, Place::At(x, y), 260.0);
                self.pop = Some(Pop::Ask(key, tree, x, y));
                Some(el)
            }
            Some(Pop::Menu(key, x, y, prio)) => {
                let r = self.row(&key);
                let el = r.map(|r| {
                    let prot = r.windows_own || r.protected;
                    if !prio {
                        let title = r.menu_title();
                        // `h('div',{class:'mhead',text:..,title:p.path})`
                        let path = r.path.as_ref().map(|p| p.display().to_string());
                        let list = [
                            match &path {
                                Some(p) => Row::HeadTitled(&title, p),
                                None => Row::Head(&title),
                            },
                            Row::Item(It::icon("xend", "End task").disabled(prot)),
                            Row::Item(It::icon("tree", "End process tree").disabled(prot)),
                            Row::Sep,
                            Row::Item(It::icon("fold", "Open file location")),
                            Row::Item(It::icon("prio", "Set priority").right(Right::Sub)),
                        ];
                        mitems::menu(cx, K_MENU, &list, Place::At(x, y), 214.0)
                    } else {
                        let title = format!("Priority \u{b7} {}", r.name);
                        let mut list = vec![Row::Head(&title)];
                        list.extend(Priority::MENU.iter().map(|p| Row::Item(It::tick(p.name(), *p == r.priority))));
                        mitems::menu(cx, sub(K_MENU, "p"), &list, Place::At(x, y), 214.0)
                    }
                });
                self.pop = Some(Pop::Menu(key, x, y, prio));
                el
            }
            None => None,
        };
        let t = self.toast.clone().map(|(t, at)| toast::toast(cx, K_TOAST, &t, at, false));
        let _ = K_TMP;
        match (pop, t) {
            (None, None) => None,
            (p, t) => Some(El::block().abs(0.0, 0.0, 0.0, 0.0).no_hit().children(p).children(t)),
        }
    }
    fn popup_dismiss(&mut self) {
        self.pop = None;
    }
    fn describe(&self) -> String {
        let rows = self.rows();
        format!(
            "rows={} first={} sort={:?}{} windows={} query={}",
            rows.len(),
            rows.first().map(|r| r.name.as_str()).unwrap_or(""),
            self.sort.0,
            if self.sort.1 { "↓" } else { "↑" },
            self.show_windows,
            self.query
        )
    }
}

#[allow(dead_code)]
fn _icons(_: &HashMap<PathBuf, Arc<Pixels>>) {}

#[cfg(test)]
mod tests;
