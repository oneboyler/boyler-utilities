//! The Storage page's boxes, from menu-v22.html (each part quotes the CSS it copies).

use taffy::style::{AlignItems, JustifyContent};

use bu_storage::classify::FileType;
use bu_storage::cleanup::{CleanKind, PartState};
use bu_storage::health::TempLevel;
use bu_storage::scan::RowKind;
use bu_storage::MediaKind;

use super::*;
use crate::anim::{Bezier, EASE};
use crate::gfx::{sh, Font, Rgba};
use crate::pages::network::temp::{self, Shape};
use crate::ui::el::{key, lh, sub, Cursor, Key, RADIUS_PILL};
use crate::ui::pieces::button::{self, Kind};
use crate::ui::pieces::mitems::{self, It, Place, Row};
use crate::ui::pieces::udlg;
use crate::ui::pieces::tip::{self, Rq};
use crate::ui::pieces::{self, bits, group, inote, link, mbtn, ptl, reset, seg};
use crate::ui::{cmix, ACC, AMBER, CTL, CTL_H, FG, FG2, FG3, GREEN, GRP, HAIR, HOV, ICO, LVT, RED, SEL, TRK};

const F11: Font = Font::new(11.0, 400);
/// Keys of the drive-health rows' tipped parts (page-local; no click behind them).
const K_HLIFE: Key = key("sto.hl");
const SCAN: Bezier = Bezier::new(0.45, 0.0, 0.55, 1.0);

/// The type colours (`ST_TYPES`).
fn type_color(t: FileType) -> Rgba {
    Rgba::hex(match t {
        FileType::Games => 0x5b8cff,
        FileType::Videos => 0xb072ff,
        FileType::Apps => 0x2fc7b6,
        FileType::Pictures => 0xffad4a,
        FileType::Documents => 0xff6f91,
        FileType::WindowsOther => 0x8d96a8,
    })
}

pub fn page(s: &mut Storage, cx: &mut Cx) -> Vec<El> {
    // Order 055: this build says whether a sweep / a shimmer is on screen (`Storage::tick` keeps frames coming for them)
    s.clock.set(cx.now);
    s.sweep_on.set(false);
    s.shim_on.set(false);
    let mut v = vec![pieces::header("Storage", None).margin(-s.shift, 2.0, 8.0, 2.0), tiles(s, cx)];
    v.extend(using(s, cx));
    v.extend(clean_up(s, cx));
    v.extend(health(s, cx));
    v
}

// ================================================================ drive tiles

/// The drive tiles `.pcg` > `.ptl.dtl` (shared `ptl::grid` + `ptl::dtl`): each tile = `.pch` (drive letter `.pcl`, label `.pcn`, disk
/// icon `.dtk`), `.pcm` (free space `.pcv` + `small` unit, "free of N GB" `.pcx`) and the used-space bar `.dbar` (amber when low).
fn tiles(s: &mut Storage, cx: &mut Cx) -> El {
    let mut v = Vec::new();
    for (i, d) in s.drives.iter().enumerate() {
        let k = idx(K_DRV, i);
        let on = d.info.letter == s.drv;
        let free = d.info.total_bytes.saturating_sub(d.used_bytes);
        let fs = gbf(free);
        let (num, unit) = fs.rsplit_once(' ').unwrap_or((fs.as_str(), ""));
        let letter = format!("{}:", d.info.letter);
        let label = if d.info.label.is_empty() { "Local Disk".to_string() } else { d.info.label.clone() };
        let of = format!("free of {}", gbf(d.info.total_bytes));
        let t = ptl::Tile {
            label: &letter,
            name: Some(&label),
            right: Some(ptl::dtk(if d.info.media == MediaKind::Hdd { "hdd" } else { "ssd" })),
            value: Some((num, if unit.is_empty() { None } else { Some(unit) })),
            extra: Some(ptl::pcx(cx, &[(of.as_str(), false)], true)),
            below: vec![ptl::dbar(cx, k, d.used_fraction as f32, d.low_space)],
        };
        // `title:d.m+' · '+d.kind` ("Samsung 990 PRO · NVMe SSD")
        v.push(ptl::dtl(cx, k, ptl::Span::Two, t, on).title(&d.hover()));
    }
    ptl::grid(v)
}

// ================================================================ What's using <drive>

fn drive_icon(s: &Storage) -> &'static str {
    match s.drives.iter().find(|d| d.info.letter == s.drv).map(|d| d.info.media) {
        Some(MediaKind::Hdd) => "hdd",
        _ => "ssd",
    }
}

fn using(s: &mut Storage, cx: &mut Cx) -> Vec<El> {
    let l = s.drv;
    let done = matches!(s.scan_of(l), Scan::Done { .. });
    let used = s.drives.iter().find(|d| d.info.letter == l).map(|d| d.used_bytes).unwrap_or(0);
    let mut head = Vec::new();
    if done {
        head.push(bits::ghs(&gbf(used)));
    }
    // `useSeg.style.visibility = scanned ? '' : 'hidden'` (still takes its place)
    let sg = seg::seg(cx, K_SEG, &["File types", "Folders"], if s.view == View::Types { 0 } else { 1 }, true);
    let sg = if done { sg } else { sg.opacity(0.0) };
    let mut out = vec![mbtn::gh_with(&format!("What’s using {l}:"), head, vec![sg])];
    // the group box `.grp.usg{overflow:hidden}`
    let body: Vec<El> = match s.scan_of(l) {
        Scan::Idle => vec![idle_state(s, cx)],
        Scan::Running { ctl, started } => {
            let p = ctl.progress();
            s.sweep_on.set(true);
            vec![scan_state(cx, &s.clock, l, used, p.bytes, cx.now - started)]
        }
        Scan::Done { result, .. } => {
            let r = result.clone();
            let mut kids = if s.view == View::Types {
                types_view(s, cx, &r)
            } else {
                // Order 069: the Folders view's small Folders | Files switch on top, then one of the two lists
                let mut v = vec![fmode_bar(s, cx)];
                v.extend(if s.fmode == FMode::Files { files_view(s, cx) } else { folders_view(s, cx, &r) });
                v
            };
            // `anim(list,[{opacity:0,transform:'translateX('+dir*10+'px)'},{opacity:1,transform:'translateX(0px)'}],{duration:220,
            // easing:EASE_OUT})` on the list (the box's last child)
            if let Some((t0, dir)) = s.list_anim {
                let t = ((cx.now - t0) / LIST_MS).clamp(0.0, 1.0);
                if t < 1.0 && !cx.rm {
                    cx.st.busy = true;
                    let p = crate::anim::EASE_OUT.ease(t) as f32;
                    if let Some(last) = kids.pop() {
                        kids.push(last.opacity(p).translate(dir * 10.0 * (1.0 - p), 0.0));
                    }
                }
            }
            kids
        }
    };
    out.push(group::grp(body).clip());
    // the foot `.gf.usf{display:flex;align-items:baseline;gap:4px}` "Measured at 21:37 ·" + Measure again (`.lnk.hide` = opacity 0,
    // its line stays)
    let (when, show) = match s.scan_of(l) {
        // the owner Oct 8: "note when a scan was last done" - how long ago (test pictures: the drawing's clock time)
        Scan::Done { when, at, .. } => (if s.env.frozen { format!("Measured at {when} ·") } else { format!("Measured {} at {when} ·", crate::keep::ago(*at)) }, true),
        _ => (String::new(), false),
    };
    let again = link::link(cx, K_AGAIN, "Measure again", 11.0);
    out.push(
        El::row()
            .items(AlignItems::BASELINE)
            .gap(4.0)
            .margin(7.0, 12.0, 0.0, 12.0)
            .child(El::text(when, F11, FG3(), lh(11.0, 1.4)))
            .child(if show { again } else { again.opacity(0.0) }),
    );
    out
}

/// v22 idle: `.scw.idle{display:flex;flex-direction:column;align-items:center;gap:10px;padding:26px 0 24px}`
/// `.sci{width:34px;height:34px;border-radius:10px;background:var(--ctl);box-shadow:inset 0 0 0 .5px var(--hair)} svg{18px;stroke:var(--fg2);
/// stroke-width:1.4}` `.sct{font-size:12.5px;font-weight:500 (idle);color:var(--fg2)}` + Measure (`.cbtn.acc.sm`)
fn idle_state(s: &Storage, cx: &mut Cx) -> El {
    El::col()
        .items(AlignItems::CENTER)
        .gap(10.0)
        .pad(26.0, 0.0, 24.0, 0.0)
        .child(El::block().size(34.0, 34.0).radius(10.0).bg(CTL()).inset(&[sh(0.0, 0.0, 0.0, 0.5, HAIR())]).place_center().child(El::icon(drive_icon(s), 18.0, 1.4, FG2())))
        .child(El::text("Not measured yet", Font::new(12.5, 500), FG2(), lh(12.5, 1.35)))
        .child(button::cbtn(cx, K_MEASURE, "Measure", Kind::Primary, true, false, 0.0))
}

/// While measuring: `.scw{gap:9px;padding:30px 0 28px}` "Measuring C:…" (`.sct` 12.5 / 600) + the calm sweep `.scan{width:180px;
/// height:4px;border-radius:2px;background:var(--trk)} i{width:40%;background:var(--acc);animation:scan 1.1s cubic-bezier(.45,0,.55,1)
/// infinite}`. Order 022 adds what the drawing leaves out: how far it is (`.scw small{font-size:11px;color:var(--fg3)}`) and Stop.
fn scan_state(cx: &mut Cx, clock: &std::rc::Rc<std::cell::Cell<f64>>, l: char, used: u64, seen: u64, ms: f64) -> El {
    // Order 055: no `st.busy` (that built the whole page every frame for the whole walk): the sweep is a live box that reads
    // the frame's time when it is painted
    let clock = clock.clone();
    let sweep = El::paint(move |g, (x, y, w, h)| {
        let t = SCAN.ease((clock.get() % 1100.0) / 1100.0) as f32;
        let pw = w * 0.4;
        g.push_clip_rr4(x, y, w, h, [2.0; 4]);
        g.fill_rr(x + pw * (-1.0 + 3.5 * t), y, pw, h, 2.0, ACC());
        g.pop_clip();
    })
    .abs(0.0, 0.0, 0.0, 0.0)
    .live();
    let secs = (ms / 1000.0).floor() as u64;
    let line = format!("{} of {} · {} s", gbf(seen.min(used.max(seen))), gbf(used), secs);
    El::col()
        .items(AlignItems::CENTER)
        .gap(9.0)
        .pad(30.0, 0.0, 28.0, 0.0)
        .child(El::text(format!("Measuring {l}:…"), Font::new(12.5, 600), FG(), lh(12.5, 1.35)))
        .child(El::block().size(180.0, 4.0).radius(2.0).bg(TRK()).clip().child(sweep))
        .child(El::text(line, Font::new(11.0, 400).tnum(), FG3(), lh(11.0, 1.35)))
        .child(button::cbtn(cx, K_STOP, "Stop", Kind::Ghost, true, false, 0.0))
}

/// File types: `.tbw{padding:12px 12px 4px}` `.tyb{display:flex;gap:2px;height:12px;border-radius:4px;overflow:hidden}` `i{min-width:3px;
/// flex-basis:0}` `.tyfree{background:var(--trk)}` `.tyhot i:not(.on){opacity:.28}` `.tbl{justify-content:space-between;margin-top:6px;
/// font-size:11px;color:var(--fg3);tabular-nums}`; rows `.tyr{min-height:34px;padding-top:5px;padding-bottom:5px;gap:10px}` `:hover{hov}`
/// `.tyd{10x10;border-radius:3px}` `.tyv{font-size:12.5px;tabular-nums;min-width:64px;text-align:right}` `.typc{font-size:11.5px;
/// color:var(--fg3);width:36px;text-align:right}`
fn types_view(s: &Storage, cx: &mut Cx, r: &ScanResult) -> Vec<El> {
    let d = s.drives.iter().find(|d| d.info.letter == s.drv);
    let used = r.types.used_bytes.or(d.map(|d| d.used_bytes)).unwrap_or(0);
    let free = r.types.free_bytes.or(d.map(|d| d.info.free_bytes)).unwrap_or(0);
    let rows: Vec<_> = r.types.rows().into_iter().filter(|t| t.bytes > 0).collect();
    let hot = rows.iter().enumerate().find(|(i, _)| cx.hovered(idx(K_TYR, *i))).map(|(i, _)| i);
    let mut bar = El::row().gap(2.0).h(12.0).radius(4.0).clip();
    for (i, t) in rows.iter().enumerate() {
        let op = cx.tr(idx(K_TYR, i), 2, if hot.is_some_and(|h| h != i) { 0.28 } else { 1.0 }, 150.0, EASE);
        // `h('i',{title:name+' · '+GBf(gb)})`
        bar = bar.child(
            El::block()
                .grow(t.bytes as f32)
                .min_w(3.0)
                .style(|st| st.flex_basis = taffy::prelude::length(0.0))
                .bg(type_color(t.ty))
                .opacity(op)
                .key(sub(idx(K_TYR, i), "seg"))
                .title(&format!("{} \u{b7} {}", t.ty.name(), gbf(t.bytes))),
        );
    }
    let fop = cx.tr(K_TYR, 2, if hot.is_some() { 0.28 } else { 1.0 }, 150.0, EASE);
    // `h('i',{class:'tyfree',title:'Free · '+GBf(d.tot-d.used)})`
    bar = bar.child(
        El::block()
            .grow(free as f32)
            .min_w(3.0)
            .style(|st| st.flex_basis = taffy::prelude::length(0.0))
            .bg(TRK())
            .opacity(fop)
            .key(sub(K_TYR, "free"))
            .title(&format!("Free \u{b7} {}", gbf(free))),
    );
    let legend = El::row()
        .justify(JustifyContent::SPACE_BETWEEN)
        .margin(6.0, 0.0, 0.0, 0.0)
        .child(El::text(format!("{} used", gbf(used)), Font::new(11.0, 400).tnum(), FG3(), lh(11.0, 1.35)))
        .child(El::text(format!("{} free", gbf(free)), Font::new(11.0, 400).tnum(), FG3(), lh(11.0, 1.35)));
    let out = vec![El::block().pad(12.0, 12.0, 4.0, 12.0).child(bar).child(legend)];
    // the rows in their own box (the drawing's `list` - the part that slides in)
    let mut list = Vec::new();
    for (i, t) in rows.iter().enumerate() {
        let k = idx(K_TYR, i);
        let hv = cx.hover_t(k, 120.0, EASE);
        let pct = ((t.bytes as f64 / used.max(1) as f64) * 100.0).round().max(1.0);
        // `.usg>.tbw+div>.row.first::before{display:block}`: the first row keeps its line under the bar
        list.push(
            group::row(false, vec![
                El::block().size(10.0, 10.0).none().radius(3.0).bg(type_color(t.ty)),
                El::col().flex1().child(El::text(t.ty.name(), Font::new(13.0, 400), FG(), lh(13.0, 1.35)).ellipsis()),
                El::text(gbf(t.bytes), Font::new(12.5, 400).tnum(), FG(), lh(12.5, 1.35)).min_w(64.0).align(crate::gfx::Align::Right).none(),
                El::text(format!("{pct} %"), Font::new(11.5, 400).tnum(), FG3(), lh(11.5, 1.35)).w(36.0).align(crate::gfx::Align::Right).none(),
            ])
            .min_h(34.0)
            .pad(5.0, 12.0, 5.0, 12.0)
            .gap(10.0)
            .bg(HOV().mul_a(hv))
            .key(k),
        );
    }
    let mut out = out;
    out.push(El::block().children(list));
    out
}

/// The folder glyph, filled: `.fdr .fdi svg{width:16px;height:16px;fill:rgba(255,200,61,.18);stroke:#e8b34a;stroke-width:1.3}`
/// (`.lockd`: no fill, stroke var(--fg3)); a file: the `file` glyph (viewBox 16).
fn fdi(file: bool, locked: bool) -> El {
    // light (Order 033): `#sw.light .fdr .fdi svg{fill:rgba(232,163,58,.2);stroke:#c98a1c}`
    let (fill, stroke) = if locked {
        (None, FG3())
    } else if crate::ui::is_light() {
        (Some(Rgba::rgba(232, 163, 58, 0.2)), Rgba::hex(0xc98a1c))
    } else {
        (Some(Rgba::rgba(255, 200, 61, 0.18)), Rgba::hex(0xe8b34a))
    };
    let shapes = if file {
        vec![Shape { d: "M4 2.5h5.2L12 5.3v8.2H4z", fill, stroke: Some((1.3, stroke)) }, Shape { d: "M9 2.5v3h3", fill: None, stroke: Some((1.3, stroke)) }]
    } else {
        vec![Shape { d: "M2.75 6A1.5 1.5 0 0 1 4.25 4.5h3.3l1.7 1.7h6.5a1.5 1.5 0 0 1 1.5 1.5v6.8a1.5 1.5 0 0 1-1.5 1.5H4.25a1.5 1.5 0 0 1-1.5-1.5z", fill, stroke: Some((1.3, stroke)) }]
    };
    let vb = if file { 16.0 } else { 20.0 };
    El::block().size(16.0, 16.0).none().place_center().child(temp::svg(shapes, vb, vb, 16.0, 16.0))
}

/// The small switch on top of the Folders view: `Folders | Files` (the shared `seg`, left, with the list's own side padding).
fn fmode_bar(s: &Storage, cx: &mut Cx) -> El {
    let sg = seg::seg(cx, K_FSEG, &["Folders", "Files"], if s.fmode == FMode::Files { 1 } else { 0 }, true);
    El::row().pad(8.0, 12.0, 8.0, 12.0).inset(&[sh(0.0, -1.0, 0.0, 0.0, HAIR())]).child(sg)
}

/// Files: the drive's biggest single files (Order 069), biggest first - icon, name with its folder small underneath, the size
/// bar and size; hover shows Show in folder, a right-click opens the file's menu (Show in folder / Delete to the Recycle
/// Bin). Out of the same walk as the folders: nothing is measured again.
fn files_view(s: &Storage, cx: &mut Cx) -> Vec<El> {
    let biggest = s.big_files();
    let max = biggest.first().map(|b| b.bytes).unwrap_or(1).max(1);
    let mut list = Vec::new();
    if biggest.is_empty() {
        list.push(El::text("No files found", Font::new(12.5, 400), FG3(), lh(12.5, 1.35)).pad(14.0, 12.0, 14.0, 12.0));
    }
    for (i, b) in biggest.iter().enumerate() {
        let k = idx(K_BIG, i);
        let hv = cx.hover_t(k, 120.0, EASE);
        let locked = !b.can_recycle();
        let dir = b.dir.display().to_string();
        let ttl = El::col()
            .flex1()
            .min_w(0.0)
            .child(El::text(b.name.clone(), Font::new(12.5, 400), FG(), lh(12.5, 1.35)).ellipsis().tip(&b.path().display().to_string()))
            .child(El::text(dir, F11, FG3(), lh(11.0, 1.35)).ellipsis().margin(1.0, 0.0, 0.0, 0.0));
        let w = ((b.bytes as f64 / max as f64) * 100.0).max(1.5) as f32;
        let fdb = El::block().size(96.0, 4.0).none().radius(2.0).bg(LVT()).clip().child(El::block().abs(0.0, 0.0, f32::NAN, 0.0).w_pct(w).radius(2.0).bg(ACC()).opacity(0.85));
        let fk = idx(K_BOP, i);
        let fh = cx.hover_t(fk, 120.0, EASE);
        let fop = El::block()
            .size(24.0, 24.0)
            .none()
            .radius(6.0)
            .bg(CTL_H().mul_a(fh))
            .place_center()
            .opacity(hv)
            .on_click(fk)
            .cursor(Cursor::Hand)
            .title("Show in folder")
            .child(El::icon("open", 13.0, 1.5, cmix(FG2(), FG(), fh)).no_hit());
        let r = group::row(i == 0, vec![
            fdi(true, locked),
            ttl,
            fdb,
            El::text(gbf(b.bytes), Font::new(12.0, 400).tnum(), FG2(), lh(12.0, 1.35)).w(62.0).align(crate::gfx::Align::Right).none(),
            fop,
        ])
        .min_h(44.0)
        .pad(4.0, 12.0, 4.0, 12.0)
        .gap(10.0)
        .bg(HOV().mul_a(hv))
        // a left click does nothing; the key makes the row take the right button (Ev::Context)
        .on_click(k);
        list.push(r);
    }
    vec![El::block().children(list)]
}

/// The Files row's right-click menu: the folder it is in on top, Show in folder, Delete (red; greyed for what Windows manages).
pub(super) fn file_menu_el(s: &Storage, cx: &mut Cx) -> Option<El> {
    let (f, x, y) = s.fmenu.clone()?;
    let head = f.dir.display().to_string();
    let title = f.path().display().to_string();
    let list = vec![
        Row::HeadTitled(&head, &title),
        Row::Item(It::icon("fold", "Show in folder")),
        Row::Item(It::icon("trash", "Delete").danger().disabled(!f.can_recycle())),
    ];
    Some(mitems::menu(cx, K_FMENU, &list, Place::At(x, y), 214.0))
}

/// The delete confirm (the shared `udlg`): what goes where, Cancel · Delete.
pub(super) fn file_dialog_el(s: &Storage, cx: &mut Cx) -> Option<El> {
    let (f, at) = s.fdlg.clone()?;
    let title = format!("Delete {}?", f.name);
    let line = format!("It goes to the Recycle Bin ({}), where you can restore it from. It is in {}.", gbf(f.bytes), f.dir.display());
    let footer = vec![
        button::cbtn_sized(cx, sub(K_FDLG, "no"), "Cancel", Kind::Ghost, button::DFT, false, 76.0),
        button::cbtn_sized(cx, sub(K_FDLG, "go"), "Delete", Kind::Red, button::DFT, false, 76.0),
    ];
    Some(udlg::udlg(cx, K_FDLG, &title, &line, &[], 0, None, footer, at, s.fdlg_closing))
}

/// Folders: the path line `.fcr{display:flex;align-items:center;gap:2px;height:38px;padding:0 12px 0 6px;box-shadow:inset 0 -1px 0 var(--hair)}`
/// (back `.fbk` 26 x 26, the parts `.fcb{height:24px;padding:0 6px;border-radius:5px;font-size:12.5px;color:var(--fg2)}` `.fcur{color:var(--fg);
/// font-weight:600}`, `.fsep` chevrons, `.fsz` the size) and the rows `.fdr{min-height:36px;padding-top:4px;padding-bottom:4px;gap:10px}`.
fn folders_view(s: &Storage, cx: &mut Cx, r: &ScanResult) -> Vec<El> {
    let tree = &r.tree;
    let cur = s.path.last().copied().unwrap_or(tree.root());
    let crumbs = tree.breadcrumb(cur).unwrap_or_default();
    let can_back = !s.path.is_empty();
    let bh = if can_back { cx.hover_t(K_BACK, 120.0, EASE) } else { 0.0 };
    let mut back = El::block()
        .size(26.0, 26.0)
        .none()
        .radius(6.0)
        .bg(CTL_H().mul_a(bh))
        .place_center()
        .child(El::icon("chevL", 12.0, 1.6, cmix(FG2(), FG(), bh)).no_hit());
    // `title:'Back'` (also while disabled at the drive's top: a disabled button still shows its name)
    back = if can_back { back.on_click(K_BACK).cursor(Cursor::Hand) } else { back.opacity(0.3).key(K_BACK) }.title("Back");
    let mut line = El::row().center().gap(2.0).h(38.0).pad(0.0, 12.0, 0.0, 6.0).inset(&[sh(0.0, -1.0, 0.0, 0.0, HAIR())]).min_w(0.0).child(back);
    for (i, (_, name, _)) in crumbs.iter().enumerate() {
        if i > 0 {
            line = line.child(El::block().w(10.0).none().place_center().child(El::icon("chevR", 5.0, 1.4, FG3())));
        }
        let last = i == crumbs.len() - 1;
        let k = idx(K_CRUMB, i);
        let hv = if last { 0.0 } else { cx.hover_t(k, 120.0, EASE) };
        let mut b = El::row()
            .center()
            .h(24.0)
            .pad(0.0, 6.0, 0.0, 6.0)
            .radius(5.0)
            .bg(HOV().mul_a(hv))
            .min_w(0.0)
            .child(El::text(name.clone(), Font::new(12.5, if last { 600 } else { 400 }).ls(0), if last { FG() } else { cmix(FG2(), FG(), hv) }, lh(12.5, 1.35)).ellipsis());
        if !last {
            b = b.on_click(k).cursor(Cursor::Hand);
        }
        line = line.child(b);
    }
    line = line.child(El::text(gbf(tree.size(cur).unwrap_or(0)), Font::new(11.5, 400).tnum(), FG3(), lh(11.5, 1.35)).none().ml_auto().pad(0.0, 0.0, 0.0, 10.0));
    let rows = tree.rows(cur).unwrap_or_default();
    let max = rows.first().map(|r| r.bytes).unwrap_or(1).max(1);
    let mut list = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let k = idx(K_FDR, i);
        let (deep, locked, file) = match row.kind {
            RowKind::Folder { has_subfolders, windows_own, .. } => (has_subfolders && !windows_own, windows_own, false),
            RowKind::File => (false, false, true),
            RowKind::OtherFiles { .. } => (false, false, true),
        };
        let hv = cx.hover_t(k, 120.0, EASE);
        let mut ttl = El::row().center().gap(4.0).min_w(0.0).child(
            El::text(row.name.clone(), Font::new(12.5, 400), if locked { FG2() } else { FG() }, lh(12.5, 1.35)).ellipsis().pad(0.0, 0.0, 2.0, 0.0).margin(0.0, 0.0, -2.0, 0.0),
        );
        if locked {
            // `.prk{width:14px;height:14px;color:var(--fg3)} svg{11px;stroke-width:1.3}`
            // `.prk[data-tip="Windows’ own · it manages this itself"]`
            ttl = ttl.child(El::block().size(14.0, 14.0).none().place_center().key(crate::ui::el::sub(k, "lock")).tip("Windows’ own · it manages this itself").child(El::icon("lock", 11.0, 1.3, FG3()).no_hit()));
        }
        // `.fdb{width:96px;height:4px;border-radius:2px;background:var(--lvt)} i{background:var(--acc);opacity:.85}`
        let w = ((row.bytes as f64 / max as f64) * 100.0).max(1.5) as f32;
        let fdb = El::block().size(96.0, 4.0).none().radius(2.0).bg(LVT()).clip().child(El::block().abs(0.0, 0.0, f32::NAN, 0.0).w_pct(w).radius(2.0).bg(ACC()).opacity(0.85));
        // `.fop{width:24px;height:24px;border-radius:6px;color:var(--fg2);opacity:0}` (shown on the row's hover) `svg{13px;stroke-width:1.5}`
        let fk = idx(K_FOP, i);
        let fh = cx.hover_t(fk, 120.0, EASE);
        let fop = if locked {
            El::block().size(24.0, 24.0).none()
        } else {
            El::block()
                .size(24.0, 24.0)
                .none()
                .radius(6.0)
                .bg(CTL_H().mul_a(fh))
                .place_center()
                .opacity(hv)
                .on_click(fk)
                .cursor(Cursor::Hand)
                // `title:'Open in Explorer'`
                .title("Open in Explorer")
                .child(El::icon("open", 13.0, 1.5, cmix(FG2(), FG(), fh)).no_hit())
        };
        // a folder kept from an earlier menu without its inside is being scanned again: the spinner in the chevron's place
        let sub_since = match (&s.sub, row.kind) {
            (Some((l, sid, _, since)), RowKind::Folder { id, .. }) if *l == s.drv && *sid == id => Some(*since),
            _ => None,
        };
        let chev = match sub_since {
            Some(since) => El::block().w(11.0).none().margin(0.0, -2.0, 0.0, 0.0).place_center().child(crate::ui::pieces::bits::uspin(cx, since)),
            None => El::block().w(8.0).none().margin(0.0, -2.0, 0.0, 0.0).place_center().child_if(deep, || El::icon("chevR", 5.0, 1.4, FG3())),
        };
        let mut r = group::row(i == 0, vec![
            fdi(file, locked),
            El::col().flex1().child(ttl),
            fdb,
            El::text(gbf(row.bytes), Font::new(12.0, 400).tnum(), FG2(), lh(12.0, 1.35)).w(62.0).align(crate::gfx::Align::Right).none(),
            fop,
            chev,
        ])
        .min_h(36.0)
        .pad(4.0, 12.0, 4.0, 12.0)
        .gap(10.0)
        .bg(HOV().mul_a(hv));
        r = if deep { r.on_click(k).cursor(Cursor::Hand) } else { r.key(k) };
        list.push(r);
    }
    vec![line, El::block().children(list)]
}

// ================================================================ Clean up

/// The drawing's row texts (`CLN`): icon, name, detail line, tip.
fn clean_meta(k: CleanKind) -> (&'static str, &'static str) {
    match k {
        CleanKind::RecycleBin => ("trash", "Emptying it can’t be undone"),
        CleanKind::TempFiles => ("tmpf", "Files an open app is still using are skipped"),
        CleanKind::ShaderCaches => ("cube", "Games recompile shaders on their next start: expect a stutter on the first run"),
        CleanKind::LauncherCaches => ("pad", "Only the launchers' web caches · logins and cookies are never touched, you stay logged in"),
    }
}

/// The detail line: in a picture copy (`env.frozen`) the drawing's own sample lines (they are its data); otherwise the crate's
/// line before measuring and what was found after it, worded like the drawing ("Steam 1.2 GB · Epic 0.5 GB"), with the
/// blocked parts' notes ("Windows temp folder needs admin").
pub(super) fn detail(s: &Storage, k: CleanKind) -> String {
    let row = s.plan.as_ref().and_then(|p| p.row(k));
    let sample = s.env.frozen && s.env.fake();
    let row = if sample { None } else { row };
    let Some(row) = row else {
        if sample {
            return match k {
                CleanKind::RecycleBin => "312 files on C: and D:",
                CleanKind::TempFiles => "Windows and app temp folders",
                CleanKind::ShaderCaches => "DirectX 1.1 GB · NVIDIA 1.7 GB",
                CleanKind::LauncherCaches => "Steam 1.2 GB · Epic 0.5 GB · Riot 0.2 GB",
            }
            .into();
        }
        return k.detail().into();
    };
    let notes = row.notes();
    let found = match k {
        CleanKind::RecycleBin => format!("{} files", row.items),
        CleanKind::TempFiles => "Windows and app temp folders".into(),
        _ => {
            // "DirectX 1.1 GB · NVIDIA 1.7 GB": the parts that hold something
            let mut parts: Vec<(String, u64)> = Vec::new();
            for p in row.parts.iter().filter(|p| p.bytes > 0 && p.state == PartState::Ready) {
                let name = p.launcher.map(|l| l.name).unwrap_or(p.name.as_str()).split(' ').next().unwrap_or("").to_string();
                match parts.iter_mut().find(|(n, _)| *n == name) {
                    Some(e) => e.1 += p.bytes,
                    None => parts.push((name, p.bytes)),
                }
            }
            if parts.is_empty() {
                "Nothing to clean".into()
            } else {
                parts.iter().map(|(n, b)| format!("{n} {:.1} GB", *b as f64 / 1_073_741_824.0)).collect::<Vec<_>>().join(" · ")
            }
        }
    };
    if notes.is_empty() {
        found
    } else {
        format!("{found} · {}", notes.join(" · "))
    }
}

fn clean_up(s: &mut Storage, cx: &mut Cx) -> Vec<El> {
    let measured = s.plan.is_some();
    let mut rows = Vec::new();
    let mut total = 0u64;
    let mut n_sel = 0;
    for (i, k) in CleanKind::ALL.iter().enumerate() {
        let row = s.plan.as_ref().and_then(|p| p.row(*k));
        // a row cleaned now counts down first (`GBf(from+(to-from)*k)`, 520 ms, rows 380 ms apart), then shows ✓
        let cd = s.countdown.as_ref().and_then(|c| c.progress(*k, cx.now, cx.rm).map(|p| (c, p)));
        // (Order 069: only while nothing is left in it - the measure after a clean shows what really is)
        let cleaned = s.report.as_ref().filter(|_| s.done(*k)).and_then(|r| r.rows.iter().find(|c| c.kind == Some(*k))).or_else(|| {
            cd.filter(|(_, p)| *p >= 1.0).and_then(|(c, _)| c.report.rows.iter().find(|r| r.kind == Some(*k)))
        });
        // after Measure: this row's size has arrived (`300 + i*180` ms) while the others still shimmer
        let arrived = s.staged.as_ref().and_then(|p| p.row(*k)).filter(|_| cx.rm || cx.now >= s.size_t0 + SIZE_FIRST_MS + i as f64 * SIZE_STEP_MS);
        // before measuring a row counts as having something (the drawing ticks every row at rest, greyed)
        let has = (!measured || row.is_some_and(|r| !r.is_empty())) && cleaned.is_none();
        let on = has && s.ticked.contains(k);
        if on && measured {
            total += row.map(|r| r.bytes).unwrap_or(0);
            n_sel += 1;
        }
        let (icon, tip_text) = clean_meta(*k);
        let ck = idx(K_CN, i);
        let tick = reset::tick(cx, crate::ui::el::sub(ck, "tick"), on);
        // `#sw .tkb:disabled{opacity:.35}`
        let tick = if measured && has && !s.cleaning { tick } else { tick.opacity(0.35) };
        // `.cnrow.cnempty` (measured and nothing left - empty, or cleaned): name and line in var(--fg3)
        let empty = measured && !has;
        let name_c = if empty { FG3() } else { FG() };
        // `.cnrow .lbl .ttl{gap:2px}` + the tip icon `.rq.rqi{width:16px;height:16px}` carrying the row's `data-tip` (the drawing's `CLN` tip)
        // (the drawing's row click ignores clicks on the `.rq`: it takes them itself, so the row does not tick)
        let rqk = crate::ui::el::sub(ck, "rq");
        let rq = tip::rq(cx, rqk, Rq::Info, 16.0, tip_text, false).on_click(rqk);
        let ttl = El::row().center().gap(2.0).min_w(0.0).child(El::text(k.name(), Font::new(13.0, 400), name_c, lh(13.0, 1.35)).ellipsis().pad(0.0, 0.0, 2.0, 0.0).margin(0.0, 0.0, -2.0, 0.0)).child(rq);
        let lbl = El::col().flex1().child(ttl).child(El::text(detail(s, *k), F11, if empty { FG3() } else { FG2() }, lh(11.0, 1.35)).ellipsis().margin(1.0, 0.0, 0.0, 0.0));
        // `.cnval{min-width:72px;text-align:right;font-size:13px;font-weight:600;tabular-nums}`
        let val: El = if let Some(c) = cleaned {
            // `.qfok{gap:5px;color:var(--green)} svg{12px;stroke-width:1.8}` `span{color:var(--fg2)}`, 11.5 / 500
            let t = if c.in_use_bytes > 0 { format!("{} in use", gbf(c.in_use_bytes)) } else { "Cleaned".to_string() };
            El::row().center().gap(5.0).child(El::icon("dcheck", 12.0, 1.8, GREEN())).child(El::text(t, Font::new(11.5, 500), FG2(), lh(11.5, 1.35)))
        } else if let (Some((c, p)), Some(r)) = (cd, row) {
            cx.st.busy = true;
            let from = c.rows.iter().find(|x| x.0 == *k).map(|x| x.1).unwrap_or(r.bytes) as f64;
            let to = c.report.rows.iter().find(|x| x.kind == Some(*k)).map(|x| x.in_use_bytes).unwrap_or(0) as f64;
            El::text(gbf((from + (to - from) * p) as u64), Font::new(13.0, 600).tnum(), FG(), lh(13.0, 1.35))
        } else if let Some(r) = row {
            El::text(gbf(r.bytes), Font::new(13.0, 600).tnum(), if empty { FG3() } else { FG() }, lh(13.0, 1.35))
        } else if let Some(r) = arrived {
            El::text(gbf(r.bytes), Font::new(13.0, 600).tnum(), FG(), lh(13.0, 1.35))
        } else if s.measuring {
            s.shim_on.set(true);
            shim(&s.clock)
        } else {
            El::text("—", Font::new(13.0, 600), FG3(), lh(13.0, 1.35))
        };
        let val = El::row().justify(JustifyContent::FLEX_END).min_w(72.0).none().child(val);
        let click = measured && has && !s.cleaning;
        let hv = if click { cx.hover_t(ck, 120.0, EASE) } else { 0.0 };
        let mut r = group::row(i == 0, vec![
            tick,
            El::block().size(20.0, 20.0).none().place_center().child(El::icon(icon, 18.0, 1.4, ICO())),
            lbl,
            val,
        ])
        .gap(10.0)
        .min_h(48.0)
        .bg(HOV().mul_a(hv));
        r = if click { r.on_click(ck).cursor(Cursor::Hand) } else { r.key(ck) };
        rows.push(r);
    }
    // the foot `.row.clf{min-height:50px;justify-content:space-between}` `.clnS{font-size:12px;color:var(--fg2);tabular-nums}`
    // `#sw .cbtn.cln{min-width:118px;tabular-nums}`
    let sum = if !measured {
        if s.measuring { "Measuring…".to_string() } else { "Not measured yet".to_string() }
    } else if n_sel > 0 {
        format!("{n_sel} selected")
    } else {
        "Nothing selected".to_string()
    };
    let label = if !measured {
        if s.measuring { "Measuring…".to_string() } else { "Measure".to_string() }
    } else if s.cleaning {
        "Cleaning…".to_string()
    } else if total > 0 {
        format!("Clean {}", gbf(total))
    } else {
        "Clean".to_string()
    };
    let disabled = s.measuring || s.cleaning || (measured && total == 0);
    let btn = button::cbtn(cx, K_CLN, &label, Kind::Primary, false, disabled, 118.0);
    // Order 069: "Measure again" next to the summary, always there once measured (its line stays when it is hidden: while a
    // measure or a clean runs)
    let again = link::link(cx, K_CAGAIN, "Measure again", 11.0);
    let again = if measured && !s.measuring && !s.cleaning { again } else { again.opacity(0.0) };
    let left = El::row()
        .items(AlignItems::BASELINE)
        .gap(8.0)
        .flex1()
        .child(El::text(sum, Font::new(12.0, 400).tnum(), FG2(), lh(12.0, 1.35)))
        .child(again);
    rows.push(group::row(false, vec![left, btn]).min_h(50.0).justify(JustifyContent::SPACE_BETWEEN));
    vec![group::gh("Clean up").child(bits::ghs("all drives")), group::grp(rows)]
}

/// `.shim{width:48px;height:10px;border-radius:5px;background:linear-gradient(90deg,var(--ctl) 0%,var(--ctl-h) 50%,var(--ctl) 100%);
/// background-size:200% 100%;animation:shim 1.1s linear infinite}` (from background-position 100% to -100%)
fn shim(clock: &std::rc::Rc<std::cell::Cell<f64>>) -> El {
    // Order 055: a live box (no `st.busy`): it reads the frame's time when it is painted
    let clock = clock.clone();
    El::paint(move |g, (x, y, w, h)| {
        let t = ((clock.get() % 1100.0) / 1100.0) as f32;
        // a 2w-wide gradient image slid from position 100% (offset -w) to -100% (offset +w)
        let off = -w + 2.0 * w * t;
        let shd = g.hgrad(x + off - w, y, x + off + w, y, &[(0.0, CTL()), (0.5, CTL_H()), (1.0, CTL())]);
        g.fill_rr_shader(x, y, w, h, 5.0, &shd, 1.0);
    })
    .size(48.0, 10.0)
    .none()
    .live()
}

// ================================================================ Drive health

/// `.tmp{display:inline-flex;align-items:center;gap:5px;height:18px;padding:0 7px 0 6px;border-radius:9px;background:var(--ctl);font-size:11px;
/// font-weight:600;color:var(--fg);tabular-nums}` `i{5x5;green}` `.warm i{amber}` `.hot i{red}` (75 / 85 °C)
fn tmp_pill(c: i32) -> El {
    let dot = match bu_storage::health::temp_level(c) {
        TempLevel::Ok => GREEN(),
        TempLevel::Warm => AMBER(),
        TempLevel::Hot => RED(),
    };
    El::row()
        .center()
        .gap(5.0)
        .h(18.0)
        .none()
        .pad(0.0, 7.0, 0.0, 6.0)
        .radius(9.0)
        .bg(CTL())
        .child(El::block().size(5.0, 5.0).none().radius(RADIUS_PILL).bg(dot))
        .child(El::text(format!("{c} °C"), Font::new(11.0, 600).tnum(), FG(), lh(11.0, 1.35)))
}

fn hours(h: u64) -> String {
    // toLocaleString('en-US'): 21,870
    let s = h.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    format!("{out} h")
}

/// `.hlr{display:grid;grid-template-columns:minmax(0,1fr) 66px 92px 74px 86px;align-items:center;column-gap:8px;min-height:40px;padding:0 12px}`
/// `::before` line (not on the head or the first row) · `.hlh{min-height:30px;box-shadow:inset 0 -1px 0 var(--hair);font-size:11px;font-weight:600;color:var(--fg3)}`
fn hl_grid(first: bool) -> El {
    let mut g = El::grid()
        .style(|st| {
            use taffy::prelude::*;
            st.grid_template_columns = vec![minmax(length(0.0), fr(1.0)), length(66.0), length(92.0), length(74.0), length(86.0)];
            st.gap.width = LengthPercentage::length(8.0);
        })
        .items(AlignItems::CENTER)
        .min_h(40.0)
        .pad(0.0, 12.0, 0.0, 12.0);
    if !first {
        g = g.child(El::block().abs(12.0, 0.0, 0.0, f32::NAN).h(1.0).bg(HAIR()).no_hit());
    }
    g
}

fn health(s: &Storage, cx: &mut Cx) -> Vec<El> {
    let head_f = Font::new(11.0, 600);
    let head = ["Drive", "Temp", "Life left", "Power-on", "Status"];
    let mut rows = vec![hl_grid(true).min_h(30.0).inset(&[sh(0.0, -1.0, 0.0, 0.0, HAIR())]).children(head.iter().map(|t| El::text(*t, head_f, FG3(), lh(11.0, 1.35))))];
    let mut warns = Vec::new();
    for (i, h) in s.health.iter().enumerate() {
        let letter = h.letters.first().map(|l| format!("{l}: ")).unwrap_or_default();
        let icon = if h.media == MediaKind::Hdd { "hdd" } else { "ssd" };
        // `.hln{display:flex;align-items:center;gap:8px;font-size:12.5px}` `.hlic{16 x 16} svg{15px;stroke:var(--fg2);stroke-width:1.3}`
        let name = El::row()
            .center()
            .gap(8.0)
            .min_w(0.0)
            .child(El::block().size(16.0, 16.0).none().place_center().child(El::icon(icon, 15.0, 1.3, FG2())))
            .child(El::text(format!("{letter}{}", h.model), Font::new(12.5, 400), FG(), lh(12.5, 1.35)).ellipsis());
        let temp = match h.temperature_c {
            // an inline-flex box on the cell's text baseline, `vertical-align:1px`: Chromium puts it 0.75 px under the cell's centre
            // line box top (dom_dump: 360.31 vs 359.56)
            Some(c) => El::row().child(tmp_pill(c).margin(0.75, 0.0, 0.0, 0.0)),
            None => El::text("—", Font::new(12.0, 400), FG3(), lh(12.0, 1.35)),
        };
        // `.hll{display:flex;align-items:center;gap:7px;font-size:12px;tabular-nums}` `.hlb{width:34px;height:4px;border-radius:2px;
        // background:var(--trk)} i{background:var(--green)}` · `.hlna{font-size:12px;color:var(--fg3)}`
        let life = match h.life_left_pct {
            Some(p) => El::row()
                .center()
                .gap(7.0)
                .child(El::block().size(34.0, 4.0).none().radius(2.0).bg(TRK()).clip().child(El::block().abs(0.0, 0.0, f32::NAN, 0.0).w_pct(p as f32).radius(2.0).bg(GREEN())))
                .child(El::text(format!("{p} %"), Font::new(12.0, 400).tnum(), FG(), lh(12.0, 1.35))),
            // `.hlna[data-tip="Hard drives don’t report a life %"]`
            None if h.media == MediaKind::Hdd => El::text("—", Font::new(12.0, 400), FG3(), lh(12.0, 1.35)).key(sub(idx(K_HLIFE, i), "na")).tip("Hard drives don’t report a life %"),
            None => El::text("—", Font::new(12.0, 400), FG3(), lh(12.0, 1.35)),
        };
        let hrs = El::text(h.power_on_hours.map(hours).unwrap_or_else(|| "—".into()), Font::new(12.0, 400).tnum(), FG2(), lh(12.0, 1.35));
        // `.hst{display:inline-flex;align-items:center;gap:6px;justify-self:start;height:20px;padding:0 8px 0 7px;border-radius:10px;
        // background:var(--ctl);font-size:11px;font-weight:600}` `i{6x6;green}` `.hwarn{background:rgba(255,214,10,.13)} i{amber}`
        let warn = !h.warnings.is_empty();
        let st_text = match h.warnings.len() {
            0 if h.os_status.is_none() && h.temperature_c.is_none() && h.life_left_pct.is_none() => "—".to_string(),
            0 => "Healthy".to_string(),
            1 => "1 warning".to_string(),
            n => format!("{n} warnings"),
        };
        // `.hst.hwarn[data-tip=<the warning>]`
        let status = El::row()
            .key(sub(idx(K_HLIFE, i), "st"))
            .center()
            .gap(6.0)
            .h(20.0)
            .none()
            .self_align(taffy::style::AlignSelf::CENTER)
            .style(|st| st.justify_self = Some(taffy::style::AlignItems::START))
            .pad(0.0, 8.0, 0.0, 7.0)
            .radius(10.0)
            // (light, Order 033: `#sw.light .hst.hwarn{background:rgba(255,184,0,.16)}`)
            .bg(if !warn { CTL() } else if crate::ui::is_light() { Rgba::rgba(255, 184, 0, 0.16) } else { Rgba::rgba(255, 214, 10, 0.13) })
            .child(El::block().size(6.0, 6.0).none().radius(RADIUS_PILL).bg(if warn { AMBER() } else { GREEN() }))
            .child(El::text(st_text, Font::new(11.0, 600), FG(), lh(11.0, 1.35)));
        let status = if warn { status.tip(&h.warnings.join(" · ")) } else { status };
        // `.hlh+.hlr::before{display:none}`
        rows.push(hl_grid(i == 0).child(name).child(temp).child(life).child(hrs).child(status));
        warns.extend(h.warnings.iter().cloned());
    }
    for w in warns {
        // `.inote.hlw`: the amber info icon + the wrapping warning, the hairline above it
        rows.push(inote::inote(&w, false, &inote::ROW_WRAP));
    }
    // A_039_01: a drive gave less than an admin read would (SATA SMART, Windows' reliability counters): one small
    // "Read with admin" link with the shield - read-only, only on the click; the rows stay filled until the tab closes
    if s.health_link() {
        let lbl = El::col().flex1().min_w(0.0).child(El::text("Some drive details can only be read with admin", F11, FG2(), lh(11.0, 1.35)).ellipsis());
        let shield = tip::rq(cx, sub(K_HADM, "tip"), Rq::Adm, 18.0, tip::texts::ADM, false);
        let act = if s.health_reading { El::text("Reading\u{2026}", Font::new(12.0, 400), FG3(), 16.0).none() } else { link::link(cx, K_HADM, "Read with admin", 12.0) };
        rows.push(group::row(false, vec![lbl, group::ctl(vec![shield, act]).gap(4.0)]).min_h(40.0));
    }
    vec![group::gh("Drive health"), group::grp(rows).clip()]
}
