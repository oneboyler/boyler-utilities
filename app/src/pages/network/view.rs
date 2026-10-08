//! The Network page's boxes, from menu-v22.html (each part quotes the CSS it copies).

use taffy::style::{AlignItems, JustifyContent};

use bu_network::gameregions::Game;
use bu_network::ping::PingLevel;
use bu_network::speedtest::{SpeedPhase, SpeedResult};
use bu_network::{Adapter, AdapterKind, DnsChoice, DnsCurrent, NetworkService};

use super::gauge::{self, Look};
use super::temp::{self, Shape};
use super::*;
use crate::anim::{Bezier, EASE};
use crate::gfx::{sh, Align, Font, Rgba};
use crate::ui::el::{lh, ClassPaint, Cursor, IconPaint, RADIUS_PILL};
use crate::ui::pieces::button::{self, Kind, MCFB};
use crate::ui::pieces::listrow::{self, Tile};
use crate::ui::pieces::mbtn::{self, Mb};
use crate::ui::pieces::mitems::{self, It, Right, Row};
use crate::ui::pieces::nbox;
use crate::ui::pieces::tip::{self, Rq};
use crate::ui::pieces::{self, badge, card, dropdown, fold, group, ibtn, link, reset, toast, toggle};
use crate::ui::{ACC, AMBER, CTL, FG, FG2, FG3, GREEN, HAIR, HOV, ICO, RED, VZ2, WIN_W};

const F13: Font = Font::new(13.0, 400);
const F11: Font = Font::new(11.0, 400);
const POP_EASE: Bezier = Bezier::new(0.3, 1.3, 0.5, 1.0);

/// The whole page: the children of the drawing's `.pg`.
pub fn page(n: &mut Network, cx: &mut Cx) -> Vec<El> {
    let mut v = vec![pieces::header("Network", Some(badge::live_note("Live only while this page is open"))).margin(-n.shift, 2.0, 8.0, 2.0)];
    v.push(conn_gh(n, cx));
    v.push(conn_grp(n, cx));
    v.push(wifi_card(n, cx));
    v.push(group::gh("Speed test"));
    v.push(group::grp(vec![speed(n, cx)]));
    // stopped pings (Stop, or the menu closed them) keep their last numbers: when they came
    let mut gsh = group::gh("Game servers");
    if !(n.gs_running || n.gs_preset) && !n.pills.is_empty() {
        if let Some(at) = n.pinged_at {
            gsh = gsh.child(crate::ui::pieces::bits::ghs(&format!("last pinged {}", crate::keep::ago(at))));
        }
    }
    v.push(gsh);
    v.push(games(n, cx));
    // Order 036 (the drawing's RS.net): "Reset this page · Back to how your PC was · Windows defaults"
    v.push(reset::reset_line(cx, K_RESET, Some("Windows defaults")));
    v
}

// ================================================================ Connection

fn conn_gh(n: &mut Network, cx: &mut Cx) -> El {
    let off = n.offline();
    let mut r = Vec::new();
    if off && n.conn.is_some() {
        // `.gh .offt{height:16px;padding:0 6px;border-radius:8px;background:rgba(255,69,58,.16);color:var(--red);font-size:10px;
        // font-weight:600;line-height:16px}`
        r.push(El::row().center().h(16.0).none().pad(0.0, 6.0, 0.0, 6.0).radius(8.0).bg(Rgba::rgba(255, 69, 58, 0.16)).child(El::text("Offline", Font::new(10.0, 600), RED(), 16.0)));
    }
    // DNS: `.mbtn.dnsb` (the piece draws [DNS] [current] [admin shield] [chevron]). The drawing's `dnsB.disabled=!a` (offline) has no
    // look of its own (the piece: "a button that cannot act looks as at rest"); the page's click handler ignores it.
    let cur = n.dns.as_ref().map(|d| d.current.label()).unwrap_or("Automatic");
    r.push(mbtn::mbtn(cx, K_DNS, Mb::Dns(cur), n.dns_wait).tip("Who looks up web addresses \u{b7} for the connection in use"));
    // Flush DNS -> `.done` (green check "Flushed") for 1.6 s
    let done = n.flushed_at.is_some_and(|t| cx.now - t < mbtn::DONE_MS);
    if done {
        cx.st.busy = true;
    }
    let what = if done { Mb::Done("Flushed") } else { Mb::Text("Flush DNS") };
    r.push(mbtn::mbtn(cx, K_FLUSH, what, false).tip("Forgets saved web addresses \u{b7} fixes sites that won\u{2019}t load"));
    mbtn::gh_with("Connection", vec![], r)
}

fn conn_grp(n: &mut Network, cx: &mut Cx) -> El {
    let Some(c) = n.conn.clone() else {
        return group::grp(vec![]);
    };
    let in_use = c.in_use.clone();
    let rows = c.adapters.iter().enumerate().map(|(i, a)| adapter_row(n, cx, i, a, in_use.as_deref() == Some(a.id.as_str()))).collect();
    group::grp(rows)
}

fn speed_text(bps: u64) -> String {
    let g = bps as f64 / 1e9;
    if g >= 1.0 {
        let s = format!("{g:.1}");
        format!("{} Gbps", s.trim_end_matches(".0"))
    } else {
        format!("{} Mbps", (bps as f64 / 1e6).round())
    }
}

/// The row's second line (the drawing's nwSync wording).
fn sub_line(a: &Adapter, in_use: bool, any_in_use: bool) -> String {
    if !a.enabled {
        return "Off".into();
    }
    if !a.kind.physical() {
        return a.description.clone();
    }
    if !a.connected {
        return "Not connected".into();
    }
    if in_use {
        return match (a.kind, a.link_speed_bps) {
            (AdapterKind::Ethernet, Some(b)) => format!("In use · {}", speed_text(b)),
            _ => "In use".into(),
        };
    }
    if a.kind == AdapterKind::Wifi && any_in_use {
        "Connected · takes over if Ethernet drops".into()
    } else {
        "Connected".into()
    }
}

fn adapter_icon(k: AdapterKind) -> &'static str {
    match k {
        AdapterKind::Ethernet => "eth",
        AdapterKind::Wifi => "wifi",
        AdapterKind::Vpn => "vpn",
        AdapterKind::Virtual => "vnet",
        AdapterKind::Bluetooth => "btn20",
    }
}

/// `.tti{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;padding-bottom:2px;margin-bottom:-2px}`
fn tti(t: &str, c: Rgba) -> El {
    El::text(t, F13, c, lh(13.0, 1.35)).ellipsis().pad(0.0, 0.0, 2.0, 0.0).margin(0.0, 0.0, -2.0, 0.0)
}

/// `.row.nrow{min-height:46px}`: [.dvi icon] [.lbl: name (+ " · SSID") + type badge + admin shield / its line] [.ctl: ping pill, switch]
fn adapter_row(n: &Network, cx: &mut Cx, i: usize, a: &Adapter, in_use: bool) -> El {
    let any = n.conn.as_ref().is_some_and(|c| c.in_use.is_some());
    // `.nrow.noff .dvi{opacity:.45}`
    let dvi = El::block().size(22.0, 22.0).none().place_center().child(El::icon(adapter_icon(a.kind), 22.0, 1.5, ICO())).opacity(if a.enabled { 1.0 } else { 0.45 });
    let mut ttl = El::row().center().gap(4.0).min_w(0.0).child(tti(&a.name, FG()));
    if let Some(s) = a.ssid.as_ref().filter(|_| a.kind == AdapterKind::Wifi && a.connected) {
        // `<span class="tti">&nbsp;<em>· MyHome</em></span>`: two text runs (the space in the row's colour, then the em)
        ttl = ttl.child(El::row().min_w(0.0).shrink(1.0).child(El::text("\u{a0}", F13, FG(), lh(13.0, 1.35)).none()).child(tti(&format!("· {s}"), FG2())));
    }
    if a.kind.badge() != a.name {
        // `.ntype{height:16px;margin-left:7px;padding:0 6px;border-radius:5px;background:var(--ctl);color:var(--fg2);font-size:10px;
        // font-weight:600;line-height:16px}` `.vpn{background:rgba(191,90,242,.18);color:#d39cff}` `.vir{background:rgba(142,142,147,.2)}`
        let (bg, fg) = match a.kind {
            AdapterKind::Vpn => (Rgba::rgba(191, 90, 242, 0.18), Rgba::hex(0xd39cff)),
            AdapterKind::Virtual => (Rgba::rgba(142, 142, 147, 0.2), FG2()),
            _ => (CTL(), FG2()),
        };
        ttl = ttl.child(El::row().center().h(16.0).none().margin(0.0, 0.0, 0.0, 7.0).pad(0.0, 6.0, 0.0, 6.0).radius(5.0).bg(bg).child(El::text(a.kind.badge(), Font::new(10.0, 600), fg, 16.0)));
    }
    if NetworkService::switch_action(a).needs_admin() {
        ttl = ttl.child(tip::rq(cx, sub(idx(K_TG, i), "rq"), Rq::Adm, 18.0, tip::texts::ADM, false));
    }
    let lbl = El::col().flex1().child(ttl).child(El::text(sub_line(a, in_use, any), F11, FG2(), lh(11.0, 1.35)).ellipsis().margin(1.0, 0.0, 0.0, 0.0));
    let mut ctl = Vec::new();
    if in_use {
        // `n.pingEl=h('span',{class:'ping','data-tip':'Ping to 1.1.1.1 · every second'},..)`
        ctl.push(pill(cx, idx(K_TG, 1000 + i), n.ping_ms, PillState::Live).tip("Ping to 1.1.1.1 \u{b7} every second"));
    }
    let wait = n.waiting.contains(&a.id);
    let tg = toggle::toggle(cx, idx(K_TG, i), a.enabled, false);
    // `.tg.wait{opacity:.55;pointer-events:none}`
    ctl.push(if wait { tg.opacity(0.55) } else { tg });
    group::row(i == 0, vec![dvi, lbl, group::ctl(ctl)]).min_h(46.0)
}

#[derive(Clone, Copy, PartialEq)]
enum PillState {
    /// a number with its level colour
    Live,
    /// `.ping.idle`: grey "—" (or the last number, greyed, after Stop)
    Idle,
    /// `.ping.wait`: grey "…", the dot pulsing
    Wait,
}

/// The ping pill: `.ping{display:inline-flex;align-items:center;gap:6px;height:22px;padding:0 9px 0 8px;border-radius:11px;
/// background:var(--ctl);box-shadow:inset 0 0 0 .5px var(--hair);font-size:11.5px;font-weight:600;tabular-nums}`
/// `.ping i{width:6px;height:6px;border-radius:50%;background:var(--green)}` `.mid i{amber}` `.bad i{red}`
/// `.ping b{min-width:15px;text-align:right}` `b small{margin-left:2px;font-size:10.5px;font-weight:500;color:var(--fg2)}`
fn pill(cx: &mut Cx, k: Key, ms: Option<u32>, st: PillState) -> El {
    let num_f = Font::new(11.5, 600).tnum();
    let line = lh(11.5, 1.35);
    let (dot, num_c, op) = match (st, ms) {
        (PillState::Live, Some(v)) => (
            match PingLevel::from_ms(v) {
                PingLevel::Green => GREEN(),
                PingLevel::Amber => AMBER(),
                _ => RED(),
            },
            FG(),
            1.0,
        ),
        (PillState::Wait, _) => {
            // `@keyframes pwait{0%,100%{opacity:1}50%{opacity:.3}}` 1 s ease-in-out
            cx.st.busy = true;
            let t = (cx.now % 1000.0) / 1000.0;
            let e = Bezier::new(0.42, 0.0, 0.58, 1.0).ease(if t < 0.5 { t * 2.0 } else { 2.0 - t * 2.0 }) as f32;
            (FG3(), FG3(), 1.0 - 0.7 * e)
        }
        _ => (FG3(), FG3(), 0.6),
    };
    let num = match (st, ms) {
        (PillState::Wait, _) => "…".to_string(),
        (_, Some(v)) => v.to_string(),
        _ => "—".to_string(),
    };
    let mut b = El::row().items(AlignItems::BASELINE).justify(JustifyContent::FLEX_END).min_w(15.0).child(El::text(num, num_f, num_c, line));
    if ms.is_some() && st != PillState::Wait {
        b = b.child(El::text("ms", Font::new(10.5, 500), FG2(), line).margin(0.0, 0.0, 0.0, 2.0));
    }
    // the first number of a run pops in: `anim(pill,[{scale(.9),opacity:.4},{scale(1),opacity:1}],{duration:260,easing:cubic-bezier(.3,1.3,.5,1)})`
    let pop = cx.tr(k, 7, if st == PillState::Wait { 0.0 } else { 1.0 }, if st == PillState::Wait { 0.0 } else { 260.0 }, POP_EASE);
    El::row()
        .center()
        .gap(6.0)
        .h(22.0)
        .none()
        .pad(0.0, 9.0, 0.0, 8.0)
        .radius(11.0)
        .bg(CTL())
        .inset(&[sh(0.0, 0.0, 0.0, 0.5, HAIR())])
        .key(k)
        .child(El::block().size(6.0, 6.0).none().radius(RADIUS_PILL).bg(dot).opacity(op))
        .child(b)
        .opacity(if st == PillState::Wait { 1.0 } else { 0.4 + 0.6 * pop })
        .scale(if st == PillState::Wait { 1.0 } else { 0.9 + 0.1 * pop })
}

// ================================================================ Wi-Fi networks (a fold card, like Security's)

/// The signal glyph (the drawing's wfSig): three arcs + a dot; lit bars `.wfs .on{stroke:currentColor}`, the rest
/// `.of{stroke:var(--fg3);opacity:.6}`; `.wfs svg{width:18px;height:18px;stroke-width:1.6}` in a 20 x 20 box.
fn bars(n: u8) -> El {
    // the drawing's wfSig markup word for word (`class="on"` / `"of"` per arc; the dot is `fill="currentColor" stroke="none"`)
    let cl = |on: bool| if on { "on" } else { "of" };
    let svg = format!(
        "<svg viewBox=\"0 0 20 20\"><path class=\"{}\" d=\"M2.4 8.2a11 11 0 0 1 15.2 0\"/><path class=\"{}\" d=\"M4.9 10.8a7.4 7.4 0 0 1 10.2 0\"/><path class=\"{}\" d=\"M7.4 13.3a3.9 3.9 0 0 1 5.2 0\"/><circle class=\"f\" cx=\"10\" cy=\"15.9\" r=\"1.1\"/></svg>",
        cl(n >= 4),
        cl(n >= 3),
        cl(n >= 2)
    );
    // `.wfs .on{stroke:currentColor}` `.wfs .of{stroke:var(--fg3);opacity:.6}`
    let paint = IconPaint { fill_all: false, classes: vec![("on".into(), ClassPaint::Stroke(FG())), ("of".into(), ClassPaint::Stroke(FG3())), ("f".into(), ClassPaint::Fill(FG()))] };
    El::block().size(20.0, 20.0).none().place_center().child(El::icon_svg(&svg, 18.0, 1.6, FG()).icon_paint(paint).class_op("of", 0.6).no_hit())
}

/// The drawing's `nudge(el)` ("look here"): `scale 1 -> 1.14 (at 35 %) -> 1`, 340 ms, EASE_OUT over the whole run; None when
/// over (or reduced motion: `if(RM)return`).
pub(super) fn nudge_scale(ms: f64, rm: bool) -> Option<f32> {
    if rm || !(0.0..340.0).contains(&ms) {
        return None;
    }
    let p = crate::anim::EASE_OUT.ease(ms / 340.0) as f32;
    Some(if p < 0.35 { 1.0 + 0.14 * p / 0.35 } else { 1.14 - 0.14 * (p - 0.35) / 0.65 })
}

fn wifi_card(n: &mut Network, cx: &mut Cx) -> El {
    let cur = n.wifi.iter().find(|w| w.connected).map(|w| w.ssid.clone());
    let line = format!("{} · {} nearby", cur.map(|c| format!("{c} · connected")).unwrap_or_else(|| "Not connected".into()), n.wifi.len());
    let fk = sub(K_WF, "fold");
    let cxk = sub(K_WF, "cx");
    // `.card.secc .ch` (58 px, title gap 6): the head, its count `.fcnt` and the chevron button `.cx`
    let right = vec![fold::fcnt(&n.wifi.len().to_string(), false), fold::chev(cx, cxk, n.wf_open, false)];
    let head = fold::head(cx, fk, "wifi", "Wi-Fi networks", vec![], &line, right, true, false);
    let rows: Vec<El> = n.wifi.clone().iter().enumerate().map(|(i, w)| wifi_row(n, cx, i, w)).collect();
    // `.pg>.card{margin-top:12px}`
    card::card(cx, K_WF, head, Some(El::block().children(rows)), n.wf_open, 544.0).margin(12.0, 0.0, 0.0, 0.0)
}

/// `.row.wfr{gap:11px;min-height:44px}` (`.card.secc .xin .row{padding-left:12px}`): [signal] [name + lock / its line] [.wact]
/// and, while its password field is open (`.wfr.open{flex-wrap:wrap;padding-bottom:0}`), the `.wfx` line under it.
fn wifi_row(n: &mut Network, cx: &mut Cx, i: usize, w: &WifiNetwork) -> El {
    let rk = idx(K_WROW, i);
    let open = n.wf_pw_for.as_deref() == Some(w.ssid.as_str());
    let mut ttl = El::row().center().gap(5.0).min_w(0.0).child(tti(&w.ssid, FG()));
    if w.secured {
        // `.wfl{width:14px;height:14px;color:var(--fg3)} svg{11px;stroke-width:1.3}`
        ttl = ttl.child(El::block().size(14.0, 14.0).none().place_center().child(El::icon("lock", 11.0, 1.3, FG3())));
    }
    let lbl = El::col().flex1().child(ttl).child(El::text(w.sub_line(), F11, FG2(), lh(11.0, 1.35)).ellipsis().margin(1.0, 0.0, 0.0, 0.0));
    // `.wfr .wact{display:flex;gap:8px;opacity:0;transition:opacity .12s}` shown on hover / open / the connected row
    let mut act = Vec::new();
    if w.connected {
        act.push(El::text("Connected", Font::new(11.5, 600), GREEN(), lh(11.5, 1.35)));
        act.push(link::link(cx, idx(K_WDISC, i), "Disconnect", 12.0));
    } else {
        act.push(link::link(cx, idx(K_WCONN, i), "Connect", 12.0));
    }
    if w.saved {
        // `.wfr .lnk.red{color:var(--red)}` loses to `#sw .lnk{color:var(--acc)}` (an id rule): Chromium paints Forget blue
        act.push(link::link(cx, idx(K_WFORGET, i), "Forget", 12.0));
    }
    let show = w.connected || open || cx.hovered(rk);
    let op = cx.tr(rk, 1, if show { 1.0 } else { 0.0 }, 120.0, EASE);
    let wact = El::row().center().gap(8.0).none().opacity(op).children(act);
    // `.card.secc .fcin>.row.first::before{display:block}`: inside the fold card the first row keeps its line too
    let mut r = group::row(false, vec![bars(w.bars()), lbl, wact]).gap(11.0).min_h(44.0).key(rk);
    if open {
        // `.wfx{display:flex;align-items:center;gap:8px;width:100%;padding:0 0 10px 43px}` `.wfx .nbox{flex:1;max-width:220px}`
        // `.wfx label.wauto{display:flex;align-items:center;gap:6px;font-size:11.5px;color:var(--fg2)}`
        let fld = nbox::nbox(cx, K_WPW, &n.wf_pw, "Password", &nbox::FORM, &nbox::Cue::NONE);
        let fld = match n.nudge_at.map(|t| nudge_scale(cx.now - t, cx.rm)) {
            Some(Some(s)) => {
                cx.st.busy = true;
                fld.scale(s)
            }
            _ => fld,
        };
        let auto = El::row()
            .center()
            .gap(6.0)
            .none()
            .on_click(K_WAUTO)
            .cursor(Cursor::Hand)
            .child(reset::tick(cx, sub(K_WAUTO, "box"), n.wf_auto))
            .child(El::text("Connect automatically", Font::new(11.5, 400), FG2(), lh(11.5, 1.35)));
        let no = button::cbtn(cx, K_WNO, "Cancel", Kind::Ghost, true, false, 0.0);
        let go = button::cbtn(cx, K_WGO, "Connect", Kind::Primary, true, false, 0.0);
        let wfx = El::row().center().gap(8.0).w_pct(100.0).pad(0.0, 0.0, 10.0, 43.0).child(fld).child(auto).child(no).child(go);
        r = r.wrap().pad(7.0, 12.0, 0.0, 12.0).child(wfx);
    }
    r
}

// ================================================================ Speed test

/// `fmtSN`: 100 and over whole numbers, under 100 one decimal.
pub fn fmt_sn(v: f64) -> String {
    if v >= 100.0 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.1}")
    }
}

/// "Speed test done · ↓ 899 · ↑ 105 Mb/s" (the drawing's toast, its number format).
pub fn speed_toast(r: &SpeedResult) -> String {
    format!("Speed test done · ↓ {} · ↑ {} Mb/s", fmt_sn(r.download_mbps), fmt_sn(r.upload_mbps))
}

/// The drawing's small arrows (`ARR`, viewBox 16).
fn arr(which: &str, size: f32, stroke: f32, c: Rgba) -> El {
    let d: &'static str = match which {
        "dn" => "M8 3v9.5M4.2 8.8L8 12.6l3.8-3.8",
        "up" => "M8 13V3.5M4.2 7.2L8 3.4l3.8 3.8",
        "png" => "M2 8h2.6l1.6-4 3.2 8 1.6-4H14",
        _ => "M2 9.5l2.4-3 2.4 4 2.4-6 2.4 5L14 7",
    };
    temp::svg(vec![Shape { d, fill: None, stroke: Some((stroke, c)) }], 16.0, 16.0, size, size)
}

fn speed(n: &mut Network, cx: &mut Cx) -> El {
    let s = n.speed.clone();
    let phase = s.phase;
    let png = s.run && phase == Some(SpeedPhase::Latency);
    // the gauge's value: the live number while a transfer runs (the drawing eases it: disp += (tgt-disp)*dt/110), 0 between
    // phases and while pinging (the arc runs back: x .86 per 16 ms), the download result when done (700 ms ease-out)
    let target = if s.run {
        if png {
            0.0
        } else {
            s.live
        }
    } else if s.done {
        s.down.unwrap_or(0.0)
    } else {
        0.0
    };
    let dur = if s.done && !s.run { 700.0 } else { 110.0 };
    let gv = cx.tr(K_START, 20, target as f32, dur, if s.done { Bezier::new(0.33, 1.0, 0.68, 1.0) } else { Bezier::new(0.0, 0.0, 1.0, 1.0) }) as f64;
    let png_t = cx.tr(K_START, 21, if png { 1.0 } else { 0.0 }, 250.0, EASE);
    if s.run {
        cx.st.busy = true;
    }
    let look = Look { value: gv, up: s.run && phase == Some(SpeedPhase::Upload), png: png_t };
    let arc = El::paint(move |g, (x, y, _, _)| gauge::paint(g, x, y, look)).abs(0.0, 0.0, 0.0, 0.0).no_hit();
    // the readout `.gro{position:absolute;left:0;right:0;top:66px;flex-direction:column;align-items:center;opacity:0;transform:scale(.94);
    // transition:opacity .28s ease,transform .36s cubic-bezier(.3,1.3,.5,1)}` `.run .gro,.done .gro{opacity:1;transform:none}`
    let shown = s.run || s.done;
    let ro = cx.tr(K_START, 22, if shown { 1.0 } else { 0.0 }, 280.0, EASE);
    let rs = cx.tr(K_START, 23, if shown { 1.0 } else { 0.0 }, 360.0, POP_EASE);
    let (ph_t, ph_k, ph_c) = match phase.filter(|_| s.run) {
        Some(SpeedPhase::Upload) => ("Upload", "up", VZ2()),
        Some(SpeedPhase::Latency) => ("Ping", "png", FG2()),
        _ => ("Download", "dn", ACC()),
    };
    let val = if png { s.lat.map(|m| format!("{}", m.round() as i64)).unwrap_or_else(|| "0".into()) } else { fmt_sn(gv) };
    let unit = if png { "ms" } else { "Mb/s" };
    // `.gph{gap:5px;height:16px;font-size:11px;font-weight:600;color:var(--fg2);letter-spacing:.01em}` `svg{12px;stroke-width:1.8}`
    let gph = El::row()
        .center()
        .justify(JustifyContent::CENTER)
        .gap(5.0)
        .h(16.0)
        .child(arr(ph_k, 12.0, 1.8, ph_c))
        .child(El::text(ph_t, Font::new(11.0, 600).ls(110), FG2(), lh(11.0, 1.35)));
    // `.gval{margin-top:2px;font:600 38px/44px "Segoe UI Variable Display";letter-spacing:-.025em;tabular-nums}`
    // `.gunit{font-size:11.5px;font-weight:500;color:var(--fg2);margin-top:-1px}`
    let gro = El::col()
        .abs(0.0, 66.0, 0.0, f32::NAN)
        .items(AlignItems::CENTER)
        .no_hit()
        .opacity(ro)
        .scale(0.94 + 0.06 * rs)
        .child(gph)
        .child(El::text(val, Font::display(38.0, 600).ls(-950).tnum(), FG(), 44.0).margin(2.0, 0.0, 0.0, 0.0))
        .child(El::text(unit, Font::new(11.5, 500), FG2(), lh(11.5, 1.35)).margin(-1.0, 0.0, 0.0, 0.0));
    let gau = El::block().size(226.0, 184.0).none().child(arc).child(gro).child(start_disc(cx, shown));
    // the results column
    let row = |cx: &mut Cx, k: &str, label: &str, v: Option<f64>, unit: &str, first: bool, now: bool, prev_now: bool| -> El {
        let ic = match k {
            "dn" => ACC(),
            "up" => VZ2(),
            _ => FG2(),
        };
        let nk = sub(K_START, k);
        let hb = cx.tr(nk, 1, if now { 1.0 } else { 0.0 }, 250.0, EASE);
        let txt = match v {
            Some(x) if unit == "Mb/s" => fmt_sn(x),
            Some(x) if k == "jit" => format!("{x:.1}"),
            Some(x) => format!("{}", x.round() as i64),
            None => "—".into(),
        };
        // `.srv{font:600 17px/1 "Segoe UI Variable Display";tabular-nums;letter-spacing:-.01em}` `small{font:500 11px/1;color:var(--fg2);margin-left:3px}`
        let mut srv = El::row().items(AlignItems::BASELINE).none().child(El::text(txt, Font::display(17.0, 600).ls(-170).tnum(), if v.is_some() { FG() } else { FG3() }, 17.0));
        if v.is_some() {
            srv = srv.child(El::text(unit, Font::new(11.0, 500), FG2(), 11.0).margin(0.0, 0.0, 0.0, 3.0));
        }
        // `.srr{display:flex;align-items:center;gap:9px;height:40px;padding:0 10px;border-radius:8px}` `.srr+.srr{box-shadow:inset 0 1px 0 var(--hair)}`
        // `.srr.now{background:var(--hov)}` `.srr.now+.srr{box-shadow:none}`
        let mut r = El::row()
            .center()
            .gap(9.0)
            .h(40.0)
            .pad(0.0, 10.0, 0.0, 10.0)
            .radius(8.0)
            .bg(HOV().mul_a(hb))
            .child(El::block().size(20.0, 20.0).none().place_center().child(arr(k, 15.0, 1.6, ic)))
            .child(El::text(label, Font::new(12.5, 400), FG2(), lh(12.5, 1.35)).ellipsis().flex1())
            .child(srv);
        if !first && !prev_now {
            r = r.inset(&[sh(0.0, 1.0, 0.0, 0.0, HAIR())]);
        }
        r
    };
    let now_of = |k: &str| {
        s.run
            && match phase {
                Some(SpeedPhase::Download) => k == "dn",
                Some(SpeedPhase::Upload) => k == "up",
                Some(SpeedPhase::Latency) => k == "png" || k == "jit",
                None => false,
            }
    };
    let rows = vec![
        row(cx, "dn", "Download", s.down, "Mb/s", true, now_of("dn"), false),
        row(cx, "up", "Upload", s.up, "Mb/s", false, now_of("up"), now_of("dn")),
        // what the ping goes to (the owner Oct 8): the test server's city ("Ping to Zagreb")
        row(cx, "png", &ping_label(&s), s.ping, "ms", false, now_of("png"), now_of("up")),
        row(cx, "jit", "Jitter", s.jit, "ms", false, now_of("jit"), now_of("png")),
    ];
    // `.sft{display:flex;align-items:center;gap:10px;min-height:30px;margin-top:8px;padding:0 4px 0 10px;font-size:11px;color:var(--fg3)}`
    // `.sft .btn{opacity:0;transition:opacity .25s}` `.shw{opacity:1}`
    // 15 s down + 15 s up + the ping (speedtest.net's length); the data at ~1 Gb/s
    let foot = s.foot.clone().unwrap_or_else(|| "About 35 s · up to ~2 GB of data".into());
    let again_op = cx.tr(K_AGAIN, 9, if s.done && !s.run { 1.0 } else { 0.0 }, 250.0, EASE);
    let again = button::btn(cx, K_AGAIN, "upd", "Test again", false).opacity(again_op);
    let mut ft = El::text(foot, F11, FG3(), lh(11.0, 1.35)).ellipsis().flex1();
    // done: `sFootT.title='Nearest server: Zagreb'` (the test server's city)
    if let Some(city) = s.server.as_ref().map(|i| i.city.as_str()).filter(|c| s.done && !c.is_empty()) {
        ft = ft.key(sub(K_AGAIN, "foot")).title(&format!("Nearest server: {city}"));
    }
    let sft = El::row()
        .center()
        .gap(10.0)
        .min_h(30.0)
        .margin(8.0, 0.0, 0.0, 0.0)
        .pad(0.0, 4.0, 0.0, 10.0)
        .child(ft)
        .child(again);
    let sres = El::col().flex1().children(rows).child(sft);
    // `.spd{display:flex;align-items:center;gap:18px;padding:14px 16px 14px 12px}`
    El::row().center().gap(18.0).pad(14.0, 16.0, 14.0, 12.0).child(gau).child(sres)
}

/// The Ping row's label: "Ping to <city>" once the test server is known (its provider when the city is not said).
fn ping_label(s: &Speed) -> String {
    match &s.server {
        Some(i) if !i.city.is_empty() => format!("Ping to {}", i.city),
        Some(i) if !i.provider.is_empty() => format!("Ping to {}", i.provider),
        _ => "Ping".into(),
    }
}

/// The v22 Start disc: `#sw .gstart{left:50%;top:104px;width:96px;height:96px;margin:-48px 0 0 -48px;border-radius:50%;flex-direction:column;
/// align-items:center;justify-content:center;gap:6px;background:radial-gradient(120% 90% at 50% 0%,rgba(255,255,255,.22),
/// rgba(255,255,255,.07) 55%,rgba(255,255,255,.04));backdrop-filter:blur(14px) saturate(150%);box-shadow:inset 0 1px 0 rgba(255,255,255,.34),
/// inset 0 0 0 .5px rgba(255,255,255,.24),0 0 0 .5px rgba(0,0,0,.28),0 8px 22px rgba(0,0,0,.26);font:600 14px/1 "Segoe UI Variable Display";
/// letter-spacing:-.005em}` `:hover{transform:scale(1.04);box-shadow:...42,...34,...,0 10px 26px rgba(0,0,0,.3)}` `:active{scale(.95)}`
/// `.run .gstart,.done .gstart{opacity:0;transform:scale(.6);pointer-events:none}`
fn start_disc(cx: &mut Cx, hidden: bool) -> El {
    let hv = if hidden { 0.0 } else { cx.hover_t(K_START, 300.0, POP_EASE) };
    let pr = if hidden { 0.0 } else { cx.active_t(K_START, 300.0, POP_EASE) };
    let op = cx.tr(K_START, 30, if hidden { 0.0 } else { 1.0 }, 200.0, EASE);
    let sc = cx.tr(K_START, 31, if hidden { 0.6 } else { 1.0 }, 300.0, POP_EASE);
    let scale = sc * (1.0 + 0.04 * hv) * (1.0 - 0.05 * pr);
    let sheen = El::paint(|g, (x, y, w, h)| {
        gauge::cached(g, 0x7374_6172_7464_6973, x, y, w, h, |cv| {
            use skia_safe as sk;
            // light (Order 033): `#sw.light .gstart{background:radial-gradient(120% 90% at 50% 0%,rgba(255,255,255,.95),rgba(255,255,255,.62))}`
            let light = crate::ui::is_light();
            let (colors, pos): (Vec<_>, &[f32]) = if light {
                (vec![Rgba(1.0, 1.0, 1.0, 0.95).c4(), Rgba(1.0, 1.0, 1.0, 0.62).c4()], &[0.0, 1.0])
            } else {
                (vec![Rgba(1.0, 1.0, 1.0, 0.22).c4(), Rgba(1.0, 1.0, 1.0, 0.07).c4(), Rgba(1.0, 1.0, 1.0, 0.04).c4()], &[0.0, 0.55, 1.0])
            };
            // the ellipse 120% x 90% of the box at (50%, 0): radius 115.2 in x, 86.4 in y
            let m = sk::Matrix::scale((1.0, 86.4 / 115.2));
            let shd = sk::Shader::radial_gradient(
                (48.0, 0.0),
                115.2,
                sk::gradient_shader::GradientShaderColors::ColorsInSpace(&colors, None),
                Some(pos),
                sk::TileMode::Clamp,
                None,
                Some(&(sk::Matrix::translate((48.0, 0.0)) * m * sk::Matrix::translate((-48.0, 0.0)))),
            );
            let mut p = sk::Paint::default();
            p.set_anti_alias(true);
            if let Some(shd) = shd {
                p.set_shader(shd);
            }
            cv.draw_circle((48.0, 48.0), 48.0, &p);
        });
    })
    .abs(0.0, 0.0, 0.0, 0.0)
    .no_hit();
    let a = |v: f32| Rgba(1.0, 1.0, 1.0, v);
    // light at rest: `box-shadow:inset 0 1px 0 #fff,inset 0 0 0 .5px rgba(0,0,0,.1),0 6px 18px rgba(0,0,0,.12)`; `#sw .gstart:hover`
    // (same weight, later) still brings the dark hover shadow - the transition runs between the two
    let (ins, outs) = if crate::ui::is_light() {
        (
            [sh(0.0, 1.0, 0.0, 0.0, crate::ui::cmix(a(1.0), a(0.42), hv)), sh(0.0, 0.0, 0.0, 0.5, crate::ui::cmix(Rgba(0.0, 0.0, 0.0, 0.1), a(0.34), hv))],
            [sh(0.0, 0.0, 0.0, 0.5, Rgba(0.0, 0.0, 0.0, 0.28 * hv)), sh(0.0, 6.0 + 4.0 * hv, 18.0 + 8.0 * hv, 0.0, Rgba(0.0, 0.0, 0.0, 0.12 + 0.18 * hv))],
        )
    } else {
        (
            [sh(0.0, 1.0, 0.0, 0.0, a(0.34 + 0.08 * hv)), sh(0.0, 0.0, 0.0, 0.5, a(0.24 + 0.1 * hv))],
            [sh(0.0, 0.0, 0.0, 0.5, Rgba(0.0, 0.0, 0.0, 0.28)), sh(0.0, 8.0 + 2.0 * hv, 22.0 + 4.0 * hv, 0.0, Rgba(0.0, 0.0, 0.0, 0.26 + 0.04 * hv))],
        )
    };
    let dot = El::block()
        .size(28.0, 28.0)
        .none()
        .radius(RADIUS_PILL)
        .bg(ACC())
        .inset(&[sh(0.0, 1.0, 0.0, 0.0, a(0.28))])
        .shadow(&[sh(0.0, 2.0, 6.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.25))])
        .place_center()
        .no_hit()
        // `.gstart i svg{width:9px;height:10px;margin-left:2px;fill:#fff}`
        .child(temp::svg(vec![Shape { d: "M0 .6v7.8a.5.5 0 0 0 .76.43l6.5-3.9a.5.5 0 0 0 0-.86L.76.17A.5.5 0 0 0 0 .6z", fill: Some(crate::ui::WHITE), stroke: None }], 8.0, 9.0, 9.0, 10.0).margin(0.0, 0.0, 0.0, 2.0));
    let mut b = El::col()
        .abs(113.0 - 48.0, 104.0 - 48.0, f32::NAN, f32::NAN)
        .size(96.0, 96.0)
        .radius(RADIUS_PILL)
        .items(AlignItems::CENTER)
        .justify(JustifyContent::CENTER)
        .gap(6.0)
        .backdrop(14.0, 1.8)
        .inset(&ins)
        .shadow(&outs)
        .opacity(op)
        .scale(scale)
        .child(sheen)
        .child(dot)
        .child(El::text("Start", Font::display(14.0, 600).ls(-70), FG(), 14.0).no_hit());
    if !hidden {
        b = b.on_click(K_START).cursor(Cursor::Hand);
    }
    b
}

// ================================================================ Game servers

/// The game's tile (the drawing's GSV `bg` + `g`).
fn game_tile(g: &Game) -> Tile {
    let (glyph, a, b) = match g.id {
        "val" => ("aim", 0xff7a76, 0xd83f4c),
        "cs2" => ("aim", 0xffc56b, 0xd9861c),
        "fn" => ("pad", 0xb49bff, 0x6a4ae0),
        "rl" => ("pad", 0x5ab4ff, 0x2a6ee6),
        "lol" => ("globe", 0xd9b866, 0x8f6b1f),
        "apex" => ("aim", 0xff8a6b, 0xc43a2a),
        _ => ("pad", 0xff6b5e, 0x9a2a22),
    };
    Tile::Glyph { glyph, a: Rgba::hex(a), b: Rgba::hex(b) }
}

fn games(n: &mut Network, cx: &mut Cx) -> El {
    let g = n.games[n.game].clone();
    let run = n.gs_running || n.gs_preset;
    // `.gsh{min-height:48px;gap:10px}` `.gsh .lbl.ap{gap:10px}`: [tile · .pu.w (128 px)] ... [Start / Stop]
    let lbl = El::row().center().gap(10.0).flex1().child(listrow::tile(&game_tile(&g), 24.0)).child(dropdown::dropdown(cx, K_GSPICK, g.name, Some(128.0)));
    // `gsBtn.title=GS.run?'Stop pinging':'Ping every '+<game>+' server region'`
    let go = ibtn::gsgo(cx, K_GSGO, run).title(&if run { "Stop pinging".to_string() } else { format!("Ping every {} server region", g.name) });
    let head = group::row(true, vec![lbl, group::ctl(vec![go])]).gap(10.0).min_h(48.0);
    let mut rows = vec![head];
    // `r.best.classList.toggle('on', r===best && GS.n>1)`: from the second round on; the drawing's gsStop leaves `.gsb2.on` in
    // place: "Best" stays after Stop (until another game or Start)
    let best = if n.gs_rounds >= 2 {
        bu_network::gameregions::best(g.regions.iter().map(|r| (r.server.id.as_str(), n.pills.get(&r.server.id).and_then(|p| p.ms))))
    } else {
        None
    };
    for (i, r) in g.regions.iter().enumerate() {
        let p = n.pills.get(&r.server.id).copied().unwrap_or_default();
        let st = if r.server.targets.is_empty() {
            PillState::Idle
        } else if run {
            if p.seen && p.ms.is_some() {
                PillState::Live
            } else {
                PillState::Wait
            }
        } else {
            PillState::Idle
        };
        let ms = if r.server.targets.is_empty() { None } else { p.ms };
        let mut ttl = El::row().center().gap(7.0).min_w(0.0).child(tti(&r.server.region, FG()));
        if best == Some(r.server.id.as_str()) {
            ttl = ttl.child(badge::tag("Best", badge::Tone::Green));
        }
        let lbl = El::col().flex1().child(ttl).child(El::text(r.place.clone(), F11, FG2(), lh(11.0, 1.35)).ellipsis().margin(1.0, 0.0, 0.0, 0.0));
        // `.gsl .gsr{padding-left:48px}` `.gsl .gsr::before{left:48px}` `.gsr{gap:10px;min-height:40px}`
        let pk = idx(K_GSGO, 100 + i);
        // `'data-tip':'Round trip to '+n+' · '+where`
        let pt = format!("Round trip to {} \u{b7} {}", r.server.region, r.place);
        rows.push(group::row_ex(false, 48.0, vec![lbl, group::ctl(vec![pill(cx, pk, ms, st).tip(&pt)])]).gap(10.0).min_h(40.0).pad(7.0, 12.0, 7.0, 48.0));
    }
    group::grp(rows)
}

// ================================================================ popups (window coordinates) + the toast

pub fn popup(n: &mut Network, cx: &mut Cx) -> Option<El> {
    let mut kids = Vec::new();
    match n.pop {
        Some(Pop::Dns) => kids.push(dns_menu(n, cx)),
        Some(Pop::DnsForm) => kids.push(dns_form(n, cx)),
        Some(Pop::Games) => {
            let items: Vec<dropdown::Item> = n.games.iter().enumerate().map(|(i, g)| dropdown::Item { label: g.name.to_string(), checked: i == n.game, disabled: false }).collect();
            // openMenu → placeMenu(btn, 150): under the button (+4), rounded like `Math.round(left / top)`
            let mw = n.gs_anchor.2.max(150.0);
            let (x, y) = place(n.gs_anchor, mw, 10.0 + 26.0 * items.len() as f32);
            kids.push(dropdown::menu(cx, K_GSM, &items, x, y, mw));
        }
        None => {}
    }
    if let Some((t, at)) = n.toast.clone() {
        if cx.now - at < toast::SHOW_MS + 300.0 {
            kids.push(toast::toast(cx, K_TOAST, &t, at, false));
        }
    }
    match kids.len() {
        0 => None,
        1 => kids.pop(),
        _ => Some(El::block().abs(0.0, 0.0, f32::NAN, f32::NAN).size(WIN_W, crate::ui::WIN_H).no_hit().children(kids)),
    }
}

/// placeMenu(btn, minW): under the button (+4), its left edge; kept 8 px inside the window (then right-aligned to the button).
fn place(anchor: (f32, f32, f32, f32), mw: f32, mh: f32) -> (f32, f32) {
    let (bx, by, bw, bh) = anchor;
    let mut left = bx;
    let mut top = by + bh + 4.0;
    if left + mw > WIN_W - 8.0 {
        left = (bx + bw - mw).max(8.0);
    }
    if top + mh > crate::ui::WIN_H - 8.0 {
        top = (by - mh - 4.0).max(8.0);
    }
    (left.round(), top.round())
}

fn dns_menu(n: &mut Network, cx: &mut Cx) -> El {
    let name = n.dns.as_ref().map(|d| d.adapter_name.clone()).unwrap_or_default();
    let cur = n.dns.as_ref().map(|d| d.current.clone()).unwrap_or(DnsCurrent::Automatic);
    let custom_em = match (&cur, n.dns.as_ref()) {
        (DnsCurrent::Custom, Some(d)) => d.configured.v4.first().map(|a| a.to_string()).unwrap_or_else(|| "your own".into()),
        _ => "your own".into(),
    };
    // The rows are `mitems` rows (`.mitem` + `<em>`, `.mhead`, `.msep`). The four clickable ones are built as one list so they
    // keep `idx(K_DNSM, 0..3)` (the page's click handler): `mitems::menu` would count the header and the separator as rows
    // (Automatic would become 1, Cloudflare 2, Google 3, Custom 5).
    let mut items: Vec<Row> = DnsChoice::ALL.iter().map(|c| {
        let (l, s) = c.label();
        Row::Item(It::tick(l, cur.choice() == Some(*c)).right(Right::Em(s)))
    }).collect();
    items.push(Row::Item(It::tick("Custom\u{2026}", cur == DnsCurrent::Custom).right(Right::Em(&custom_em))));
    let mut els = mitems::rows(cx, K_DNSM, &items);
    let custom = els.pop();
    let head_text = format!("DNS for {name}");
    let head = mitems::rows(cx, K_DNSM, &[Row::Head(&head_text)]);
    let sep = mitems::rows(cx, K_DNSM, &[Row::Sep]);
    let rows: Vec<El> = head.into_iter().chain(els).chain(sep).chain(custom).collect();
    let mw = 214f32.max(n.dns_anchor.2);
    let mh = 10.0 + 29.0 + 3.0 * 26.0 + 9.0 + 26.0;
    let (x, y) = place(n.dns_anchor, mw, mh);
    dropdown::menu_box(cx, K_DNSM, x, y, mw, mh, 300.0, rows)
}

fn dns_form(n: &mut Network, cx: &mut Cx) -> El {
    let name = n.dns.as_ref().map(|d| d.adapter_name.clone()).unwrap_or_default();
    // `.dnsf{width:282px;padding:4px 6px 4px}` `b{font-size:13px;font-weight:600;margin:2px 0 8px}`
    // `.dfh{font-size:10.5px;font-weight:600;color:var(--fg3);letter-spacing:.02em;margin:8px 0 5px}`
    // `.dfr{display:grid;grid-template-columns:62px minmax(0,1fr);align-items:center;gap:8px;margin-bottom:6px;font-size:12px;color:var(--fg2)}`
    let dfh = |t: &str| El::block().margin(8.0, 0.0, 5.0, 0.0).child(El::text(t, Font::new(10.5, 600).ls(210), FG3(), lh(10.5, 1.35)));
    let mut kids = vec![El::block().margin(2.0, 0.0, 8.0, 0.0).child(El::text(format!("Custom DNS for {name}"), Font::new(13.0, 600), FG(), lh(13.0, 1.35)))];
    let labels = ["Primary", "Secondary", "Primary", "Secondary"];
    let ph = ["Primary", "Secondary", "Primary (optional)", "Secondary (optional)"];
    for i in 0..4 {
        if i == 0 {
            kids.push(dfh("IPv4"));
        }
        if i == 2 {
            kids.push(dfh("IPv6"));
        }
        let k = sub(K_DNSF, DNS_FIELDS[i]);
        let f = nbox::well(cx, k, &n.dns_fields[i], ph[i], n.dns_bad[i]);
        kids.push(
            El::grid()
                .style(|s| {
                    s.grid_template_columns = vec![taffy::prelude::length(62.0), taffy::prelude::minmax(taffy::prelude::length(0.0), taffy::prelude::fr(1.0))];
                })
                .items(AlignItems::CENTER)
                .gap(8.0)
                .margin(0.0, 0.0, 6.0, 0.0)
                .child(El::text(labels[i], Font::new(12.0, 400), FG2(), lh(12.0, 1.35)))
                .child(f),
        );
    }
    // `.mcfb{display:flex;justify-content:flex-end;gap:6px}` `.dnsf .mcfb{margin-top:10px}`
    let no = button::cbtn_sized(cx, sub(K_DNSF, "cancel"), "Cancel", Kind::Ghost, MCFB, false, 0.0);
    let save = button::cbtn_sized(cx, sub(K_DNSF, "save"), "Save", Kind::Primary, MCFB, false, 0.0);
    kids.push(El::row().justify(JustifyContent::FLEX_END).gap(6.0).margin(10.0, 0.0, 0.0, 0.0).child(no).child(save));
    // `.dnsf` is a block: its children's vertical margins collapse (b's 8 + .dfh's 8 = 8; .dfr's 6 + .dfh's 8 = 8)
    let body = El::block().w(282.0).pad(4.0, 6.0, 4.0, 6.0).children(kids);
    let mw = 292.0;
    let mh = 10.0 + 8.0 + 27.5 + 2.0 * (26.0 + 2.0 * 32.0) + 36.0;
    let (x, y) = place(n.dns_anchor, mw, mh);
    dropdown::menu_box(cx, K_DNSF, x, y, 282.0, mh, 1000.0, vec![body])
}
