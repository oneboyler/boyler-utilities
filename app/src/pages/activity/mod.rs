//! The Activity tab (menu-v22 page `act`, Order 018): ONE switch "Count my activity" (off by default); on: today's screen
//! time, game time, uptime; the most-used apps as bars (Today / 7 days, 6 shown + "Show all"); the last 7 days as a chart
//! (games teal under the rest in blue); an app's menu: "Count as a game" / "Not a game", "Don't count this app".
//! Counting is crates/activity's watcher (event-driven, only while the switch is on; nothing leaves the PC) - see data.rs.
//! The drawing's JS: `ACTIVITY: your day in apps` (menu-v22.html); its CSS is quoted on every box below.

mod data;
#[cfg(test)]
mod tests;

use taffy::style::JustifyContent;

use crate::anim::{EASE, EASE_OUT};
use crate::gfx::{Font, Rgba};
use crate::pages::{Env, Page};
use crate::ui::cx::{Cx, Ev};
use crate::ui::el::{idx, key, lh, sub, El, Key};
use crate::ui::pieces::listrow::{self, Tile};
use crate::ui::pieces::mitems::{self, It, Place, Row};
use crate::ui::pieces::ptl::{self, Span};
use crate::ui::pieces::segx::{self, Label};
use crate::ui::pieces::tip::{self, Rq};
use crate::ui::pieces::{self, badge, card, group, link, toggle};
use crate::ui::{cmix, ACC, FG, FG2, FG3, HAIR, HOV, LVT, VZ2};

use bu_activity::views::{Summary, UseRow};
pub use data::Src;

const K_ON: Key = key("act.on");
const K_BODY: Key = key("act.body");
const K_RANGE: Key = key("act.range");
const K_MORE: Key = key("act.more");
const K_ALL: Key = key("act.all");
const K_UP: Key = key("act.uptip");
const K_MENU: Key = key("act.menu");

/// The rows' keys by place in the list (the test hook's `click:el:act.row3`).
fn row_key(i: usize) -> Key {
    key(&format!("act.row{i}"))
}
/// The 7-day chart's columns (`hover:el:act.col3`).
fn col_key(i: usize) -> Key {
    key(&format!("act.col{i}"))
}

/// The drawing's `hm` (rows, tips): "2 h 41 m", "1 h 05 m", "47 m".
pub fn hm(ms: u64) -> String {
    let m = (ms as f64 / 60_000.0).round() as u64;
    if m >= 60 {
        format!("{} h {:02} m", m / 60, m % 60)
    } else {
        format!("{m} m")
    }
}
/// The drawing's `hmS` (tiles): "6 h 18 m", "17 h", "47 m".
pub fn hm_s(ms: u64) -> String {
    let m = (ms as f64 / 60_000.0).round() as u64;
    match (m / 60, m % 60) {
        (0, mm) => format!("{mm} m"),
        (h, 0) => format!("{h} h"),
        (h, mm) => format!("{h} h {mm} m"),
    }
}

/// Blink: a percentage length resolves to LayoutUnits (1/64 px, floored); the drawing writes the % with toFixed(2).
fn pct_px(part: f64, whole: f64, of: f32) -> f32 {
    let pct = if whole > 0.0 { (part / whole * 100.0 * 100.0).round() / 100.0 } else { 0.0 };
    ((of as f64 * pct / 100.0 * 64.0).floor() / 64.0) as f32
}

/// The page's tile for an app: the drawing's coloured tiles for its sample apps; any other app a plain grey tile
/// (the app's own icon needs the shared "app icon by exe path" piece - PIECES_WANTED).
fn tile_of(path: &str) -> Tile {
    let p = path.to_ascii_lowercase();
    let g = |glyph: &'static str, a: u32, b: u32| Tile::Glyph { glyph, a: Rgba::hex(a), b: Rgba::hex(b) };
    // the drawing's ACTA table: glyph + linear-gradient(135deg, a, b)
    if p.ends_with("valorant.exe") {
        g("pad", 0xff7a76, 0xd83f4c)
    } else if p.ends_with("chrome.exe") {
        g("globe", 0xffd35a, 0xe6493b)
    } else if p.ends_with("discord.exe") {
        g("chat", 0x8f95ff, 0x5a5fe0)
    } else if p.ends_with("obs64.exe") {
        g("rec", 0x7a808c, 0x3a3e47)
    } else if p.ends_with("rocketleague.exe") {
        g("pad", 0x5ab4ff, 0x2a5fd6)
    } else if p.ends_with("steam.exe") {
        g("pad", 0x6f8fb8, 0x2b3f5c)
    } else if p.ends_with("explorer.exe") {
        g("fold", 0xffd35a, 0xe8a33a)
    } else if p.ends_with("spotify.exe") {
        g("note", 0x46d989, 0x1c9a5a)
    } else if p.ends_with("systemsettings.exe") {
        g("cog16", 0xa2abbd, 0x6c7487)
    } else if p.ends_with("notepad.exe") {
        g("mtxt", 0x7fb2ff, 0x3b6fd6)
    } else if p.ends_with("boylerutilities.exe") {
        g("tgl", 0x5ab4ff, 0x2a74e6)
    } else {
        g("appw", 0xa2abbd, 0x6c7487)
    }
}

/// `.xp > .xin{overflow:hidden}` through the shared `card::drop_out` while it moves; fully open it is the content itself
/// (the shared painter's clip is not pixel-snapped like Chromium's: at rest it would cut the last box's edge row).
fn fold(cx: &mut Cx, body: El, ft: (f32, f32)) -> El {
    if ft.0 >= 1.0 && ft.1 >= 1.0 {
        body
    } else {
        card::drop_out(cx, body, 548.0, ft)
    }
}

/// An app's menu, opened by a right click on its row (`Ev::Context`).
#[derive(Clone, Debug)]
struct Menu {
    path: String,
    name: String,
    game: bool,
    /// the list row it belongs to (`.arow.ctx`)
    row: usize,
    /// the pointer (window coordinates) where it was opened
    at: (f32, f32),
}

/// While the tab shows with counting on, its numbers follow the counter: a small thread wakes the menu every
/// `LIVE_MS` (one wake-up, no polling of Windows) and the page reads the summary again (the owner Oct 8: "it shows no
/// programs" - the page read it once, right when the switch went on, and never again).
const LIVE_MS: u64 = 10_000;

struct Ticker(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Ticker {
    fn start(w: crate::services::Waker) -> Ticker {
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let s = stop.clone();
        let _ = std::thread::Builder::new().name("bu-act-live".into()).spawn(move || {
            // short sleeps so a closed tab ends the thread soon
            let mut slept = 0;
            while !s.load(std::sync::atomic::Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(250));
                slept += 250;
                if slept >= LIVE_MS {
                    slept = 0;
                    w.wake();
                }
            }
        });
        Ticker(stop)
    }
}

impl Drop for Ticker {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

#[derive(Default)]
pub struct Activity {
    src: Option<Box<dyn Src>>,
    on: bool,
    sum: Option<Summary>,
    /// when the summary was last read (page ms) and the thread that wakes the page for a new one
    read_at: f64,
    ticker: Option<Ticker>,
    env_test: bool,
    /// "7 days" chosen (`S.actRange==='week'`)
    week: bool,
    /// "Show all" open
    all: bool,
    /// when the bars started growing (switch on / range changed): the drawing's actRender(true)
    grow_at: Option<f64>,
    menu: Option<Menu>,
    /// the last toast text (the frame draws the toast; this is only for `describe`)
    last_toast: Option<String>,
    err: Option<String>,
    now: f64,
}

impl Activity {
    /// The page with its source given (tests).
    pub fn with_src(src: Box<dyn Src>) -> Activity {
        let on = src.is_on();
        let mut a = Activity { src: Some(src), on, ..Activity::default() };
        a.refresh();
        a
    }

    fn refresh(&mut self) {
        self.sum = match (&mut self.src, self.on) {
            (Some(s), true) => Some(s.summary()),
            _ => None,
        };
    }

    fn rows(&self) -> Vec<UseRow> {
        match &self.sum {
            Some(s) => {
                let v = if self.week { &s.week } else { &s.today };
                v.iter().filter(|r| r.ms > 0).cloned().collect()
            }
            None => Vec::new(),
        }
    }

    fn set_on(&mut self, on: bool, now: f64) {
        let Some(src) = &mut self.src else { return };
        match src.set_on(on) {
            Ok(()) => {
                self.err = None;
                let was = self.on;
                self.on = on;
                self.refresh();
                self.read_at = now;
                if on && !was {
                    self.grow_at = Some(now);
                }
                self.ticker = (on && !self.env_test).then(|| Ticker::start(crate::services::Waker));
                if !on {
                    self.menu = None;
                }
            }
            Err(e) => self.err = Some(e.to_string()),
        }
    }

    /// A menu row was clicked: row 0 is the head, so item 1 = "Count as a game" / "Not a game", item 2 = "Don't count".
    fn pick(&mut self, i: usize, cx: &mut Cx) {
        let Some(m) = self.menu.take() else { return };
        let Some(src) = &mut self.src else { return };
        let r = if i == 1 { src.set_game(&m.path, !m.game) } else { src.dont_count(&m.path) };
        let t = match r {
            Ok(()) => {
                let t = match i {
                    1 if !m.game => format!("{} counts as a game now", m.name),
                    1 => format!("{} no longer counts as a game", m.name),
                    _ => format!("{} isn\u{2019}t counted any more \u{b7} Settings can bring it back", m.name),
                };
                self.refresh();
                t
            }
            Err(e) => e.to_string(),
        };
        cx.toast(&t);
        self.last_toast = Some(t);
    }

    /// The bars' grow animation: progress 0..1 of an element started `delay` ms after the trigger (`fill: backwards`).
    fn grow(&self, cx: &mut Cx, delay: f64, dur: f64) -> f32 {
        match self.grow_at {
            Some(t0) => {
                let x = ((cx.now - t0 - delay) / dur).clamp(0.0, 1.0);
                if x < 1.0 {
                    cx.st.busy = true;
                }
                EASE_OUT.ease(x) as f32
            }
            None => 1.0,
        }
    }

    // ---------------------------------------------------------------------------------------------------- the boxes

    /// `.pcg.actg` - three tiles: Screen time, Games, Uptime (`.pcg{display:grid;grid-template-columns:repeat(6,
    /// minmax(0,1fr));gap:8px}` + `.ptl{grid-column:span 2}` = three equal columns; `.actb>.xin>.pcg{margin-top:12px}`).
    fn tiles(&self, cx: &mut Cx, s: &Summary) -> El {
        let wk = self.week;
        let week_ms: u64 = s.last7.iter().map(|d| d.total_ms).sum();
        let games_n = s.week.iter().filter(|r| r.game && r.ms > 0).count();
        let after = |t: &str| t.split_once(" \u{b7} ").map(|x| x.1.to_string()).unwrap_or_default();
        let (scr_v, scr_x) = if wk {
            (hm_s(week_ms), format!("last 7 days \u{b7} {} a day", hm_s(week_ms / 7)))
        } else {
            (hm_s(s.screen_today_ms), after(&s.screen_text))
        };
        let (gam_v, gam_x) = if wk {
            (hm_s(s.games_week_ms), format!("last 7 days \u{b7} {} game{}", games_n, if games_n == 1 { "" } else { "s" }))
        } else {
            (hm_s(s.games_today_ms), format!("today \u{b7} {} in 7 days", hm_s(s.games_week_ms)))
        };
        let up_v = bu_activity::views::fmt_uptime(s.uptime_ms);
        let up_x = after(&s.uptime_text);
        // `.pch>.rq{margin-left:auto;align-self:center;width:16px;height:16px;margin-top:-2px;margin-bottom:-2px}` (the
        // piece puts the left auto and the centring; the vertical margins are the tile head's rule)
        let up_tip = "Counts from the last full restart \u{b7} with Fast Startup on, Shut down doesn\u{2019}t reset it";
        let rq = tip::rq(cx, K_UP, Rq::Info, 16.0, up_tip, false).margin(-2.0, 0.0, -2.0, 0.0);
        let tile = |cx: &Cx, label: &str, right: Option<El>, v: &str, x: &str| {
            let t = ptl::Tile { label, name: None, right, value: Some((v, None)), extra: None, below: vec![ptl::atl_line(x)] };
            ptl::ptl(cx, Span::Two, t, true)
        };
        let tiles = vec![tile(cx, "Screen time", None, &scr_v, &scr_x), tile(cx, "Games", None, &gam_v, &gam_x), tile(cx, "Uptime", Some(rq), &up_v, &up_x)];
        // `.actb>.xin>.pcg{margin-top:12px}`
        ptl::grid(tiles).margin(12.0, 0.0, 0.0, 0.0)
    }

    /// One "Most used" row: `.row.arow{gap:12px;min-height:40px;padding-top:5px;padding-bottom:5px}`
    /// `.anm{display:flex;align-items:center;gap:10px;width:200px;flex:none;min-width:0;font-size:13px}` (tile, name `.tti`,
    /// the teal `.tag.gtag` "Game"), `.abar{flex:1;min-width:0;height:6px;border-radius:3px;background:var(--lvt);overflow:hidden}`
    /// `.abar i{height:100%;border-radius:3px;background:var(--acc)}` (`.arow.game .abar i{background:var(--vz2)}`),
    /// `.atm{flex:none;width:72px;text-align:right;font-size:12px;color:var(--fg2);font-variant-numeric:tabular-nums}`.
    fn row(&self, cx: &mut Cx, i: usize, r: &UseRow, max: u64) -> El {
        // .tti{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;padding-bottom:2px;margin-bottom:-2px}
        let name = El::text(r.name.clone(), Font::new(13.0, 400), FG(), lh(13.0, 1.35)).ellipsis().min_w(0.0).shrink(1.0).pad(0.0, 0.0, 2.0, 0.0).margin(0.0, 0.0, -2.0, 0.0);
        let anm = El::row()
            .center()
            .gap(10.0)
            .w(200.0)
            .none()
            .min_w(0.0)
            .child(listrow::tile(&tile_of(&r.path), 24.0))
            .child(name)
            .children(r.game.then(|| badge::tag("Game", badge::Tone::Teal)));
        // the bar: width % of the longest (toFixed(2)), growing from the left (scaleX 0 -> 1, .38 s after 60 + i x 30 ms)
        let full = pct_px(r.ms as f64, max as f64, 228.0);
        let g = self.grow(cx, 60.0 + i as f64 * 30.0, 380.0);
        let fw = full * g;
        let fc = if r.game { VZ2() } else { ACC() };
        // the fill inside the track's overflow:hidden (radius 3), both on pixel-snapped boxes like Blink paints them
        let fill = El::paint(move |gx, (x, y, w, h)| {
            let (tx, ty, tw, th) = gx.snap(x, y, w, h);
            let (fx, fy, fw2, fh) = gx.snap(x, y, fw, h);
            if fw2 > 0.0 {
                gx.push_clip_rr4(tx, ty, tw, th, [3.0; 4]);
                gx.fill_rr(fx, fy, fw2, fh, 3.0f32.min(fw2 / 2.0), fc);
                gx.pop_clip();
            }
        })
        .abs(0.0, 0.0, 0.0, 0.0);
        let bar = El::block().flex1().h(6.0).radius(3.0).bg(LVT()).child(fill);
        let tm = El::text(hm(r.ms), Font::new(12.0, 400).tnum(), FG2(), lh(12.0, 1.35)).w(72.0).none().align(crate::gfx::Align::Right);
        let open = self.menu.as_ref().map(|m| m.row == i).unwrap_or(false);
        let mut row = group::row(i == 0, vec![anm, bar, tm]).min_h(40.0).pad(5.0, 12.0, 5.0, 12.0).on_click(row_key(i));
        if open {
            // .arow.ctx{background:var(--hov)}
            row = row.bg(HOV());
        }
        row
    }

    /// "Most used": `.gh` + `.ghr` with the Today / 7 days switch, `.grp.aul{overflow:hidden}` (6 rows, the rest in a fold),
    /// `.gf.afoot{margin-top:6px}` with "Show all N" / "Show less" (`#sw .afoot .lnk{font-size:11.5px;line-height:15px}`).
    fn most_used(&self, cx: &mut Cx) -> El {
        let rows = self.rows();
        let max = rows.first().map(|r| r.ms).unwrap_or(0);
        let built: Vec<El> = rows.iter().enumerate().map(|(i, r)| self.row(cx, i, r, max)).collect();
        let mut it = built.into_iter();
        let mut kids: Vec<El> = it.by_ref().take(bu_activity::views::MOST_USED_SHOWN).collect();
        let rest: Vec<El> = it.collect();
        let n = rows.len();
        // .gh .ghr{margin-left:auto;display:flex;align-items:center;gap:10px;font-weight:400}
        let range = segx::seg_ex(cx, K_RANGE, &[Label::Text("Today"), Label::Text("7 days")], Some(self.week as usize), &segx::SM);
        let gh = group::gh("Most used").child(El::row().center().gap(10.0).ml_auto().child(range));
        let mut wrap = El::block().child(gh);
        if kids.is_empty() {
            // nothing counted yet (just switched on): say what happens - and that there is no older history to show
            // (the owner Oct 8: "my guess is it needs new data and can't take windows's or can it?" - Windows keeps per-app use
            // only in its SRUM database, readable with admin rights and locked while Windows runs: not usable here)
            kids.push(
                El::text(
                    "Nothing counted yet \u{2014} the app in front is counted from now on. Windows keeps no app-time history this app can read.",
                    Font::new(12.0, 400),
                    FG2(),
                    lh(12.0, 1.35),
                )
                .wrapping()
                .pad(12.0, 12.0, 12.0, 12.0),
            );
        }
        if !rest.is_empty() {
            let ft = card::fold_t(cx, K_MORE, self.all);
            kids.push(fold(cx, El::block().children(rest), ft));
        }
        wrap = wrap.child(group::grp(kids).clip());
        if n > bu_activity::views::MOST_USED_SHOWN {
            let label = if self.all { "Show less".to_string() } else { format!("Show all {n}") };
            let mut l = link::link(cx, K_ALL, &label, 11.5);
            if let crate::ui::el::Content::Text(t) = &mut l.content {
                t.lh = 15.0;
            }
            // .gf{margin:7px 12px 0;line-height:1.4} -> the line box 15.39 px around the 15 px button
            wrap = wrap.child(El::block().margin(6.0, 12.0, 0.0, 12.0).h(lh(11.0, 1.4)).child(El::row().child(l)));
        }
        wrap
    }

    /// "Last 7 days": `.gh` + the legend (`.aleg{display:flex;align-items:center;gap:6px;font-size:11px;color:var(--fg2)}`
    /// `.aleg i{width:8px;height:8px;border-radius:2px}` `.g{background:var(--vz2)}` `.o{margin-left:8px;background:var(--acc);
    /// opacity:.5}`) and the chart box `.grp.achwrap{position:relative;height:156px}`.
    fn last7(&self, cx: &mut Cx, s: &Summary) -> El {
        let f11 = Font::new(11.0, 400);
        let leg = El::row()
            .center()
            .gap(6.0)
            .child(El::block().size(8.0, 8.0).none().radius(2.0).bg(VZ2()))
            .child(El::text("Games", f11, FG2(), lh(11.0, 1.35)))
            .child(El::block().size(8.0, 8.0).none().radius(2.0).bg(ACC()).opacity(0.5).margin(0.0, 0.0, 0.0, 8.0))
            .child(El::text("Other apps", f11, FG2(), lh(11.0, 1.35)));
        let gh = group::gh("Last 7 days").child(El::row().center().gap(10.0).ml_auto().child(leg));
        // the scale: 10 h like the drawing (MAXD = 600 min); a longer day raises it in steps of 5 h (labels follow)
        let longest = s.last7.iter().map(|d| d.total_ms).max().unwrap_or(0);
        let maxd_min = (longest as f64 / 60_000.0 / 300.0).ceil().max(2.0) * 300.0;
        let maxd = maxd_min * 60_000.0;
        // .achgrid{position:absolute;left:44px;right:16px;top:16px;bottom:30px;display:flex;flex-direction:column;
        //   justify-content:space-between} .achgrid i{height:1px;background:var(--hair)}
        // .achgrid i[data-h]::before{right:calc(100% + 8px);top:-7px;font-size:10px;line-height:14px;color:var(--fg3)}
        let line = |label: Option<String>| {
            let mut l = El::block().h(1.0).bg(HAIR());
            if let Some(t) = label {
                let w = cx.g.text_width(&t, Font::new(10.0, 400));
                l = l.child(El::text(t, Font::new(10.0, 400), FG3(), 14.0).abs(-8.0 - w, -7.0, f32::NAN, f32::NAN));
            }
            l
        };
        let top_h = (maxd_min / 60.0).round() as u64;
        let grid = El::col()
            .abs(44.0, 16.0, 16.0, 30.0)
            .justify(JustifyContent::SPACE_BETWEEN)
            .no_hit()
            .child(line(Some(format!("{top_h} h"))))
            .child(line(Some(format!("{} h", top_h / 2))))
            .child(line(None));
        // .achart{position:absolute;left:44px;right:16px;top:16px;bottom:10px;display:flex}
        let mut chart = El::row().abs(44.0, 16.0, 16.0, 10.0);
        for (i, d) in s.last7.iter().enumerate() {
            let k = col_key(i);
            let hv = cx.hover_t(k, 150.0, EASE);
            let g = self.grow(cx, i as f64 * 35.0, 420.0);
            // .acbar{flex:1;width:24px;display:flex;flex-direction:column;justify-content:flex-end} (110 px tall here)
            // .acbar .acot{border-radius:5px 5px 0 0;background:var(--acc);opacity:.5} .acbar .acgm{background:var(--vz2)}
            // heights: (minutes / MAXD x 100).toFixed(2) % of the bar; .acol:hover .acbar{filter:brightness(1.18)}
            let gm = pct_px(d.games_ms as f64, maxd, 110.0) * g;
            let ot = pct_px((d.total_ms - d.games_ms) as f64, maxd, 110.0) * g;
            let br = |c: Rgba| cmix(c, Rgba((c.0 * 1.18).min(1.0), (c.1 * 1.18).min(1.0), (c.2 * 1.18).min(1.0), c.3), hv);
            // border-radius:5px 5px 0 0 (El has one radius): painted here on the pixel-snapped box like Blink does
            let c = br(ACC());
            let acot = El::paint(move |g, (x, y, w, h)| {
                let (sx, sy, sw, sh) = g.snap(x, y, w, h);
                if sw > 0.0 && sh > 0.0 {
                    let r = 5.0f32.min(sw / 2.0).min(sh);
                    g.push_clip_rr4(sx, sy, sw, sh, [r, r, 0.0, 0.0]);
                    g.fill_rect(sx, sy, sw, sh, c);
                    g.pop_clip();
                }
            })
            .h(ot)
            .opacity(0.5);
            let acgm = El::block().h(gm).bg(br(VZ2()));
            let bar = El::col().flex1().w(24.0).justify(JustifyContent::FLEX_END).child(acot).child(acgm);
            // .acday{flex:none;height:20px;line-height:20px;font-size:10.5px;color:var(--fg3)} .today{color:var(--fg);font-weight:600}
            let day: String = d.label.chars().take(2).collect();
            let (df, dc) = if d.today { (Font::new(10.5, 600), FG()) } else { (Font::new(10.5, 400), FG3()) };
            // .acol{flex:1;min-width:0;display:flex;flex-direction:column;align-items:center}
            // its tip (data-tip): "Wed 30 Sep · 4 h 10 m · games 1 h 50 m" - the frame draws it above the column
            let label = if d.today { "Today".to_string() } else { d.label.clone() };
            let text = format!("{label} \u{b7} {} \u{b7} games {}", hm(d.total_ms), hm(d.games_ms));
            let col = El::col().flex1().center().key(k).tip(&text).child(bar).child(El::text(day, df, dc, 20.0).h(20.0).none());
            chart = chart.child(col);
        }
        let wrap = group::grp(vec![grid, chart]).h(156.0);
        El::block().child(gh).child(wrap)
    }
}

impl Page for Activity {
    fn id(&self) -> &'static str {
        "act"
    }
    fn name(&self) -> &'static str {
        "Activity"
    }
    fn icon(&self) -> &'static str {
        "chart"
    }
    fn open(&mut self, env: &Env, now: f64) {
        // nothing slow: the fake is memory; the real one reads settings.tsv (and resumes counting if it was left on)
        let (src, err): (Box<dyn Src>, Option<String>) = if env.fake() {
            (Box::new(data::Fake::demo()), None)
        } else if env.real_read {
            (Box::new(data::ReadOnly::open()), None)
        } else {
            let (r, e) = data::Real::open();
            (Box::new(r), e)
        };
        *self = Activity::with_src(src);
        self.err = err;
        self.now = now;
        self.read_at = now;
        self.env_test = env.test;
        if self.on {
            // the drawing's first actSync with the switch on: the bars grow in
            self.grow_at = Some(now);
            if !env.test {
                self.ticker = Some(Ticker::start(env.waker()));
            }
        }
    }
    /// At app start: counting goes on where it was left (the switch is saved) - not only once the tab is opened
    /// (the owner Oct 8: "it toggles off when i close the program"). Normal runs only.
    fn background(&self, env: &Env) -> Option<Box<dyn crate::pages::Background>> {
        if !env.test {
            let _ = data::resume_if_on();
        }
        None
    }
    fn close(&mut self) {
        // the page's state goes; the watcher (if on) keeps counting - it is the switch's, not the tab's
        *self = Activity::default();
    }
    fn build(&mut self, cx: &mut Cx) -> Vec<El> {
        self.now = cx.now;
        // the ticker woke the menu: the counter's numbers again (cheap: the watcher's memory, no disk)
        if self.on && cx.now - self.read_at >= LIVE_MS as f64 - 500.0 {
            self.refresh();
            self.read_at = cx.now;
        }
        let status = match (&self.err, &self.sum) {
            (Some(e), _) => e.clone(),
            (None, Some(s)) => s.status.clone(),
            (None, None) => "Most-used apps, game time and uptime \u{b7} off".into(),
        };
        // the card: `.grp.card` with `.row.ch.first` (icon, "Count my activity" + its line, the switch)
        let head = card::card_head("chart", "Count my activity", Some(&status), vec![group::ctl(vec![toggle::toggle(cx, K_ON, self.on, false)])]);
        let card_el = card::card(cx, sub(K_ON, "card"), head, None, false, 548.0);
        let mut kids = vec![pieces::header(self.name(), None), card_el];
        // `.xp.actb` - everything below the card drops out while the switch is on
        let ft = card::fold_t(cx, K_BODY, self.on);
        let body = match self.sum.clone() {
            Some(s) => El::block().child(self.tiles(cx, &s)).child(self.most_used(cx)).child(self.last7(cx, &s)),
            None => El::block(),
        };
        kids.push(fold(cx, body, ft));
        // `.gf.actq{margin-top:7px}`
        kids.push(group::gf(bu_activity::views::PRIVACY_LINE));
        kids
    }
    fn event(&mut self, ev: &Ev, cx: &mut Cx) {
        let now = cx.now;
        // the drawing's controls are <button>s: Enter / Space on the focused one = a click (keyboard use; the test hook's
        // "enter" reaches page elements this way - its "click" can't, see the report)
        if let Ev::Key(k, 0x0D | 0x20) = ev {
            // (Order 045: used - the frame's own Enter / Space on a Tab-focused control does not click it again)
            cx.used = true;
            return self.event(&Ev::Click(*k), cx);
        }
        match ev {
            Ev::Click(k) if *k == K_ON => self.set_on(!self.on, now),
            Ev::Click(k) if *k == K_ALL => self.all = !self.all,
            Ev::Click(k) => {
                if let Some(i) = (0..2).find(|&i| idx(K_RANGE, i) == *k) {
                    let wk = i == 1;
                    if wk != self.week {
                        self.week = wk;
                        self.grow_at = Some(now);
                    }
                } else if let Some(i) = (1..3).find(|&i| idx(K_MENU, i) == *k) {
                    // the menu's rows: 0 = the head, 1 = count as a game, 2 = don't count
                    self.pick(i, cx);
                }
            }
            // the drawing's `contextmenu` on a row: its menu at the pointer
            Ev::Context(k, x, y) => {
                if let Some(i) = (0..64).find(|&i| row_key(i) == *k) {
                    if let Some(r) = self.rows().get(i) {
                        self.menu = Some(Menu { path: r.path.clone(), name: r.name.clone(), game: r.game, row: i, at: (*x, *y) });
                    }
                }
            }
            _ => {}
        }
    }
    fn popup(&mut self, cx: &mut Cx) -> Option<El> {
        let m = self.menu.clone()?;
        // the drawing's menuAt(row, x, y, 200); a click beside it closes it (the frame calls `popup_dismiss`)
        let list = [Row::Head(&m.name), Row::Item(It::icon("pad", if m.game { "Not a game" } else { "Count as a game" })), Row::Item(It::icon("x", "Don\u{2019}t count this app"))];
        Some(mitems::menu(cx, K_MENU, &list, Place::At(m.at.0, m.at.1), 200.0))
    }
    fn popup_dismiss(&mut self) {
        self.menu = None;
    }
    fn describe(&self) -> String {
        let rows = self.rows();
        format!(
            "on={} counting={} range={} all={} rows={} first={} menu={} toast={} err={}",
            self.on as u8,
            self.src.as_ref().map(|s| s.counting()).unwrap_or(false) as u8,
            if self.week { "week" } else { "day" },
            self.all as u8,
            rows.len(),
            rows.first().map(|r| r.name.as_str()).unwrap_or("-"),
            self.menu.as_ref().map(|m| m.name.as_str()).unwrap_or("-"),
            self.last_toast.as_deref().unwrap_or("-"),
            self.err.as_deref().unwrap_or("-"),
        )
    }
}
