//! The Audio page's live parts, painted the way the drawing's canvas / CSS paints them (number for number from menu-v22,
//! carried over from test A's pixel-proven painter, Order 003): the level smoothing, the device rows' level pill (the
//! volume slider's track, `drawPill`), and the app tile with CSS `filter: grayscale()` for a muted app.

use std::collections::HashMap;
use std::sync::Arc;

use skia_safe as sk;

use crate::anim::EASE;
use crate::gfx::{sh, Gfx, Rgba, Shadow};
use crate::icons::Icons;
use crate::png::Pixels;
use crate::ui::el::El;
use crate::ui::{LVT, VZ1, VZ2, WHITE};

/// One smoothed level with its held peak (the drawing's per-frame constants made frame-rate independent).
#[derive(Default, Clone, Copy, Debug)]
pub struct Level {
    pub l: f32,
    pub pk: f32,
    pub pt: f64,
}

fn per_dt(k: f32, dt: f64) -> f32 {
    1.0 - (1.0 - k).powf((dt / (1000.0 / 60.0)) as f32)
}

impl Level {
    /// `att` / `rel` = the drawing's per-frame approach (devices .28 / .06, apps .42 / .13), `hold` ms the peak waits,
    /// `decay` per frame after it, `floor` = snaps to 0 below it.
    #[allow(clippy::too_many_arguments)]
    pub fn step(&mut self, target: f32, now: f64, dt: f64, att: f32, rel: f32, hold: f64, decay: f32, floor: f32) {
        let t = target.clamp(0.0, 1.0);
        let k = if t > self.l { att } else { rel };
        self.l += (t - self.l) * per_dt(k, dt);
        if self.l < floor {
            self.l = 0.0;
        }
        if self.l >= self.pk {
            self.pk = self.l;
            self.pt = now + hold;
        } else if now > self.pt {
            self.pk = (self.pk - decay * (dt / (1000.0 / 60.0)) as f32).max(self.l);
        }
    }
    pub fn device(&mut self, target: f32, now: f64, dt: f64) {
        self.step(target, now, dt, 0.28, 0.06, 650.0, 0.007, 0.002);
    }
    pub fn app(&mut self, target: f32, now: f64, dt: f64) {
        self.step(target, now, dt, 0.42, 0.13, 520.0, 0.006, 0.003);
    }
    pub fn moving(&self) -> bool {
        self.l > 0.0 || self.pk > 0.0
    }
}

/// Order 055: an app row's held-peak dot fades in and out (`.lvl` dot, .3 s `ease`, 0 <-> .9). It used to be a transition of
/// the built page (every flip during music built the page and ran frames at the screen's rate); now it is stepped with the
/// levels (`at` is read by the live pass), so a peak showing or hiding repaints only the meter.
#[derive(Default, Clone, Copy, Debug)]
pub struct PeakFade {
    from: f32,
    to: f32,
    t0: f64,
    seen: bool,
}

impl PeakFade {
    /// The dot's opacity at `now`.
    pub fn at(&self, now: f64) -> f32 {
        self.from + (self.to - self.from) * EASE.ease((now - self.t0) / 300.0) as f32
    }
    /// The dot should show (`on`) or not: a new target starts from where the fade is now; the first call jumps (a row that
    /// is new, or a picture, shows it at once - as the transition did on its first build).
    pub fn set(&mut self, on: bool, now: f64) {
        let to = if on { 0.9 } else { 0.0 };
        if !self.seen {
            *self = PeakFade { from: to, to, t0: now, seen: true };
        } else if to != self.to {
            *self = PeakFade { from: self.at(now), to, t0: now, seen: true };
        }
    }
    /// Still fading at `now`?
    pub fn busy(&self, now: f64) -> bool {
        self.from != self.to && now - self.t0 < 300.0
    }
}

/// The slim level bar under an app's slider (`slider::level_bar_with`, painted number for number) whose dot opacity is read
/// at paint time too: `now()` = (level, peak, dot opacity).
pub fn level_bar_live(now: impl Fn() -> (f32, f32, f32) + 'static, c1: Rgba, c2: Rgba) -> El {
    El::paint(move |g, (x, y, w, _)| {
        let (level, peak, pk_op) = now();
        g.fill_rr(x, y, w, 3.0, 1.5, LVT());
        let lvl = level.clamp(0.0, 1.0);
        if lvl > 0.0 {
            let br = g.hgrad(x, 0.0, x + w, 0.0, &[(0.0, c1), (1.0, c2)]);
            let o = 0.45 + 0.55 * (lvl * 2.4).min(1.0);
            g.push_layer(1.0, Some((x, y, w * lvl, 3.0, 1.5)));
            g.fill_rr_shader(x, y, w, 3.0, 1.5, &br, o);
            g.pop_layer();
        }
        if pk_op > 0.001 {
            g.fill_circle(x + w * peak.clamp(0.0, 1.0) - 1.5, y + 1.5, 1.5, c2.mul_a(pk_op));
        }
    })
    .size(0.0, 3.0)
    .live()
}

/// The device row's canvas (`.dvs .vis`, w x 30): a faint pill, a quiet accent fill up to the knob, the live level
/// glowing inside that fill (accent -> teal), a thin sheen, the held peak tick. The drawing's drawPill.
pub fn level_pill(g: &Gfx, (x, y, w, h): (f32, f32, f32, f32), vol: f32, lv: Level) {
    let th = 8.0;
    let py = y + (h - th) / 2.0;
    let x0 = x + 2.0;
    let kx = 8.0 + (w - 16.0) * vol.clamp(0.0, 1.0);
    let vw = th.max(kx - 2.0);
    let l = lv.l;
    let fw = th.max(vw * l);
    // nothing the canvas paints reaches outside its box
    let (cx, cy, cw, ch) = g.snap(x, y, w, h);
    g.push_clip(cx, cy, cw, ch);
    g.fill_rr(x0, py, w - 4.0, th, th / 2.0, LVT());
    // the drawing's `hexA(--vz1, light ? .2 : .26)` (Order 033)
    g.fill_rr(x0, py, vw, th, th / 2.0, VZ1().a(if crate::ui::is_light() { 0.2 } else { 0.26 }));
    if l > 0.004 {
        let alpha = 0.5 + 0.5 * (l * 2.5).min(1.0);
        let br = g.hgrad(x, 0.0, x + w, 0.0, &[(0.0, VZ1()), (1.0, VZ2())]);
        // canvas 2D: shadowBlur (4 + 12 L) -> sigma half of it; the shadow first, then the gradient pill, both at globalAlpha
        g.fill_rr_blur(x0, py, fw, th, th / 2.0, VZ2().a(0.18 + 0.5 * l).mul_a(alpha), (4.0 + 12.0 * l) / 2.0);
        g.fill_rr_shader(x0, py, fw, th, th / 2.0, &br, alpha);
        if fw > 10.0 {
            g.fill_rr(x0 + 3.0, py + 1.5, fw - 6.0, 2.0, 1.0, Rgba(1.0, 1.0, 1.0, 0.1 + 0.12 * l));
        }
    }
    let p = lv.pk;
    if p - l > 0.02 {
        let a = ((p - l) * 8.0).min(1.0) * 0.8;
        g.fill_rr(x0 + vw * p - 1.5, py - 1.0, 3.0, th + 2.0, 1.5, VZ2().a(a));
    }
    g.pop_clip();
}

/// The app tile's outer shadow: `0 1px 2px rgba(0,0,0,.22)`.
const TILE_SH: [Shadow; 1] = [sh(0.0, 1.0, 2.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.22))];

/// What an app tile shows: the drawing's gradient + white glyph, or the app's own icon.
#[derive(Clone)]
pub enum Face {
    Glyph { glyph: &'static str, a: Rgba, b: Rgba },
    Icon(Arc<Pixels>),
}

/// The 24 px app tile (`.ait`). A muted app (`.mxr.mu .ait{filter:grayscale(1);opacity:.42}`, transition .25 s) gets its
/// grayscale + opacity on its element (`El::color_filter`, `El::opacity`).
pub fn tile(g: &Gfx, t: &Face, (x, y): (f32, f32)) {
    // Blink paints the tile (gradient included) and its <svg> from the pixel-snapped box
    let (x, y, _, _) = g.snap(x, y, 24.0, 24.0);
    match t {
        Face::Glyph { glyph, a, b } => {
            let br = g.hgrad(x, y, x + 24.0, y + 24.0, &[(0.0, *a), (1.0, *b)]);
            g.box_shadows(x, y, 24.0, 24.0, 6.0, &TILE_SH, false);
            g.fill_rr_shader(x, y, 24.0, 24.0, 6.0, &br, 1.0);
            g.inset_shadows(x, y, 24.0, 24.0, 6.0, &[sh(0.0, 0.0, 0.0, 0.5, Rgba(1.0, 1.0, 1.0, 0.24)), sh(0.0, 1.0, 0.0, 0.0, Rgba(1.0, 1.0, 1.0, 0.18))]);
            g.push_layer(1.0, Some((x, y, 24.0, 24.0, 6.0)));
            GLYPHS.with(|icons| icons.draw(g, glyph, x + 5.0, y + 5.0, 14.0, 1.6, WHITE, &|_| 1.0));
            g.pop_layer();
        }
        Face::Icon(px) => {
            ICONS.with(|c| {
                let k = Arc::as_ptr(px) as usize;
                let mut c = c.borrow_mut();
                if !c.contains_key(&k) {
                    if let Some(i) = crate::png::to_image(px) {
                        c.insert(k, i);
                    }
                }
                if let Some(img) = c.get(&k) {
                    g.draw_image_rect(img, x, y, 24.0, 24.0);
                }
            });
        }
    }
}

thread_local! {
    /// the drawing's ICON table for the tiles' glyphs (painted inside the tile's own paint call)
    static GLYPHS: Icons = Icons::new();
    /// app icons made into images once (dropped when the menu closes: `reset_caches`)
    static ICONS: std::cell::RefCell<HashMap<usize, sk::Image>> = std::cell::RefCell::new(HashMap::new());
}

pub fn reset_caches() {
    ICONS.with(|c| c.borrow_mut().clear());
}

/// bu-audio's icon (premultiplied BGRA rows) as the painter's pixels.
pub fn pixels(i: &bu_audio::Icon) -> Arc<Pixels> {
    Arc::new(Pixels { w: i.w, h: i.h, data: i.bgra.clone() })
}

// ------------------------------------------------------------------ the fake sound (test copies that are not frozen)
/// The drawing's fake sound per app (music keeps a beat, voice talks and pauses, the game bursts, system sounds blip)
/// and the fake voice on the input - so a test copy's meters move like the drawing's.
pub struct FakeSound {
    seed: u64,
    apps: HashMap<String, (f32, f64, f64)>,
    beat_t: f64,
    beat: f32,
    voice: (f32, f64, i32),
}

impl Default for FakeSound {
    fn default() -> Self {
        FakeSound { seed: 0x2545_f491_4f6c_dd1d, apps: HashMap::new(), beat_t: 0.0, beat: 0.0, voice: (0.0, 0.0, 0) }
    }
}

impl FakeSound {
    fn rnd(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        (self.seed >> 40) as f32 / (1u64 << 24) as f32
    }
    /// The raw level of one app now (before its volume): `mode` from the drawing's APPS.
    pub fn app(&mut self, name: &str, mode: &str, now: f64) -> f32 {
        let (mut tgt, mut nt, mut bt) = self.apps.get(name).copied().unwrap_or((0.0, 0.0, 0.0));
        let t = match mode {
            "blip" => {
                if now > nt {
                    nt = now + 3200.0 + self.rnd() as f64 * 3000.0;
                    bt = now + 170.0;
                }
                if now < bt {
                    0.5
                } else {
                    0.0
                }
            }
            "none" => 0.0,
            _ => {
                if now > nt {
                    let (r1, r2) = (self.rnd(), self.rnd() as f64);
                    match mode {
                        "music" => {
                            tgt = 0.52 + r1 * 0.34;
                            nt = now + 110.0 + r2 * 90.0;
                        }
                        "voice" => {
                            let talk = r1 < 0.62;
                            tgt = if talk { 0.3 + r2 as f32 * 0.5 } else { 0.0 };
                            nt = now + if talk { 80.0 + r2 * 170.0 } else { 260.0 + r2 * 700.0 };
                        }
                        _ => {
                            let hit = r1 < 0.3;
                            tgt = if hit { 0.6 + r2 as f32 * 0.35 } else { 0.16 + r2 as f32 * 0.14 };
                            nt = now + if hit { 60.0 + r2 * 90.0 } else { 120.0 + r2 * 240.0 };
                        }
                    }
                }
                tgt
            }
        };
        self.apps.insert(name.to_string(), (tgt, nt, bt));
        t
    }
    /// What the apps play together (`sum`), with the music's soft beat.
    pub fn output(&mut self, sum: f32, now: f64) -> f32 {
        if now > self.beat_t {
            self.beat_t = now + 469.0;
            self.beat = 1.0;
        }
        self.beat *= 0.9;
        (sum * 0.6).min(1.0) * (0.84 + 0.24 * self.beat)
    }
    /// The fake voice: syllables, words, pauses.
    pub fn input(&mut self, now: f64) -> f32 {
        let (env, until, left) = self.voice;
        if now >= until {
            if left > 0 && env > 0.1 {
                let r = self.rnd() as f64;
                self.voice = (0.06, now + 30.0 + r * 40.0, left);
            } else if left > 0 {
                let (r1, r2) = (self.rnd(), self.rnd() as f64);
                self.voice = (0.38 + r1 * 0.55, now + 90.0 + r2 * 130.0, left - 1);
            } else {
                let (r1, r2, r3) = (self.rnd(), self.rnd(), self.rnd() as f64);
                let n = 1 + (r1 * 4.0) as i32;
                let pause = if r2 < 0.18 { 900.0 + r3 * 900.0 } else { 220.0 + r3 * 380.0 };
                self.voice = (0.0, now + pause, n);
            }
        }
        self.voice.0 * (0.9 + 0.1 * ((now / 41.0).sin() as f32))
    }
}
