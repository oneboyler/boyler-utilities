//! Motion helpers: CSS cubic-bezier easing, tweens and the drawing's keyframe shapes.

/// A CSS `cubic-bezier(x1,y1,x2,y2)` timing function, solved the way browsers do (Newton + bisection on x).
#[derive(Clone, Copy, Debug)]
pub struct Bezier {
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
}

impl Bezier {
    pub const fn new(x1: f64, y1: f64, x2: f64, y2: f64) -> Self {
        Bezier { x1, y1, x2, y2 }
    }
    fn cx(&self, t: f64) -> f64 {
        let (a, b, c) = (1.0 - 3.0 * self.x2 + 3.0 * self.x1, 3.0 * self.x2 - 6.0 * self.x1, 3.0 * self.x1);
        ((a * t + b) * t + c) * t
    }
    fn cy(&self, t: f64) -> f64 {
        let (a, b, c) = (1.0 - 3.0 * self.y2 + 3.0 * self.y1, 3.0 * self.y2 - 6.0 * self.y1, 3.0 * self.y1);
        ((a * t + b) * t + c) * t
    }
    fn dx(&self, t: f64) -> f64 {
        let (a, b, c) = (1.0 - 3.0 * self.x2 + 3.0 * self.x1, 3.0 * self.x2 - 6.0 * self.x1, 3.0 * self.x1);
        (3.0 * a * t + 2.0 * b) * t + c
    }
    /// Eased progress for linear progress `x` in 0..1.
    pub fn ease(&self, x: f64) -> f64 {
        if x <= 0.0 {
            return 0.0;
        }
        if x >= 1.0 {
            return 1.0;
        }
        let mut t = x;
        for _ in 0..8 {
            let e = self.cx(t) - x;
            if e.abs() < 1e-7 {
                return self.cy(t);
            }
            let d = self.dx(t);
            if d.abs() < 1e-6 {
                break;
            }
            t -= e / d;
        }
        let (mut lo, mut hi) = (0.0, 1.0);
        t = x;
        for _ in 0..40 {
            let v = self.cx(t);
            if (v - x).abs() < 1e-7 {
                break;
            }
            if v < x {
                lo = t;
            } else {
                hi = t;
            }
            t = (lo + hi) / 2.0;
        }
        self.cy(t)
    }
}

pub const EASE: Bezier = Bezier::new(0.25, 0.1, 0.25, 1.0); // CSS `ease`
pub const EASE_IN: Bezier = Bezier::new(0.42, 0.0, 1.0, 1.0); // CSS `ease-in`
pub const EASE_OUT_CSS: Bezier = Bezier::new(0.0, 0.0, 0.58, 1.0); // CSS `ease-out`
pub const EASE_OUT: Bezier = Bezier::new(0.2, 0.8, 0.2, 1.0); // the drawing's EASE_OUT
pub const ACCEL: Bezier = Bezier::new(0.4, 0.0, 1.0, 1.0); // close / page-out
pub const SPRING_POP: Bezier = Bezier::new(0.3, 1.35, 0.5, 1.0); // dock tiles pop in
pub const DOT_GLIDE: Bezier = Bezier::new(0.3, 1.22, 0.5, 1.0); // the active dot
pub const RISE_A: Bezier = Bezier::new(0.2, 0.8, 0.3, 1.0); // open rise, first part
pub const RISE_B: Bezier = Bezier::new(0.45, 0.0, 0.4, 1.0); // open rise, settle
pub const SWITCH: Bezier = Bezier::new(0.3, 0.7, 0.2, 1.0); // switch knob

/// Linear progress of an animation that starts at `start` (ms), waits `delay` and runs `dur` ms.
pub fn prog(now: f64, start: f64, delay: f64, dur: f64) -> f64 {
    if dur <= 0.0 {
        return 1.0;
    }
    ((now - start - delay) / dur).clamp(0.0, 1.0)
}

pub fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// A value that moves to a new target with a CSS `transition` (duration + easing), from wherever it is now.
#[derive(Clone, Copy, Debug)]
pub struct Tween {
    from: f64,
    to: f64,
    start: f64,
    dur: f64,
    delay: f64,
    ease: Bezier,
}

impl Tween {
    pub fn new(v: f64) -> Self {
        Tween { from: v, to: v, start: 0.0, dur: 0.0, delay: 0.0, ease: EASE }
    }
    pub fn value(&self, now: f64) -> f64 {
        let p = prog(now, self.start, self.delay, self.dur);
        lerp(self.from, self.to, self.ease.ease(p))
    }
    pub fn target(&self) -> f64 {
        self.to
    }
    pub fn set(&mut self, now: f64, to: f64, dur: f64, ease: Bezier) {
        self.set_delayed(now, to, dur, 0.0, ease);
    }
    pub fn set_delayed(&mut self, now: f64, to: f64, dur: f64, delay: f64, ease: Bezier) {
        if (to - self.to).abs() < 1e-9 {
            return;
        }
        self.from = self.value(now);
        self.to = to;
        self.start = now;
        self.dur = dur;
        self.delay = delay;
        self.ease = ease;
    }
    pub fn jump(&mut self, v: f64) {
        *self = Tween::new(v);
    }
    pub fn busy(&self, now: f64) -> bool {
        now < self.start + self.delay + self.dur
    }
}

/// The open rise: translateY 16 -> -2.5 (at 60 %, ease RISE_A) -> 0 (ease RISE_B), 340 ms.
pub fn open_rise(t_ms: f64) -> f64 {
    let p = (t_ms / 340.0).clamp(0.0, 1.0);
    if p < 0.6 {
        lerp(16.0, -2.5, RISE_A.ease(p / 0.6))
    } else {
        lerp(-2.5, 0.0, RISE_B.ease((p - 0.6) / 0.4))
    }
}

/// The dock click bounce (.42 s ease-out per keyframe segment): 0, -4 @30 %, 0 @55 %, -1.5 @75 %, 0.
pub fn bounce(t_ms: f64) -> f64 {
    let p = (t_ms / 420.0).clamp(0.0, 1.0);
    let k = [(0.0, 0.0), (0.30, -4.0), (0.55, 0.0), (0.75, -1.5), (1.0, 0.0)];
    for w in k.windows(2) {
        let (a, b) = (w[0], w[1]);
        if p <= b.0 {
            let q = (p - a.0) / (b.0 - a.0);
            return lerp(a.1, b.1, EASE_OUT_CSS.ease(q));
        }
    }
    0.0
}

/// The small "look here" nudge: scale 1 -> 1.14 @35 % -> 1, 340 ms, EASE_OUT over the whole run.
pub fn nudge(t_ms: f64) -> f64 {
    let p = EASE_OUT.ease((t_ms / 340.0).clamp(0.0, 1.0));
    if p < 0.35 {
        lerp(1.0, 1.14, p / 0.35)
    } else {
        lerp(1.14, 1.0, (p - 0.35) / 0.65)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bezier_endpoints_and_linear() {
        let lin = Bezier::new(0.0, 0.0, 1.0, 1.0);
        for i in 0..=10 {
            let x = i as f64 / 10.0;
            assert!((lin.ease(x) - x).abs() < 1e-5);
        }
        assert_eq!(EASE_OUT.ease(0.0), 0.0);
        assert_eq!(EASE_OUT.ease(1.0), 1.0);
        // overshooting curve goes above 1 in the middle
        assert!((0..100).map(|i| SPRING_POP.ease(i as f64 / 100.0)).fold(0.0, f64::max) > 1.0);
    }
    #[test]
    fn rise_shape() {
        assert!((open_rise(0.0) - 16.0).abs() < 1e-9);
        assert!((open_rise(204.0) + 2.5).abs() < 1e-6);
        assert!(open_rise(340.0).abs() < 1e-9);
    }
}
