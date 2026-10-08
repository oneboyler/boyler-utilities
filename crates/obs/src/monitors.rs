//! Monitors (monitors.c): "Monitor 1" = Windows' main display right now, the others count up from 2, left to right; OBS's
//! display-capture `monitor_id` -> our monitor; which monitor a "Show on" choice means (popup.c `pick_monitor`).

use crate::settings::{W_MON1, W_MON2, W_NONE, W_SAME};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn w(&self) -> i32 {
        self.right - self.left
    }
    pub fn h(&self) -> i32 {
        self.bottom - self.top
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Mon {
    /// \\.\DISPLAY1
    pub dev: String,
    /// the device interface path (\\?\DISPLAY#ABC1234#5&1a2b&0&UID1#{...}) = what OBS stores as monitor_id
    pub iface: String,
    pub rc: Rect,
    pub work: Rect,
    pub primary: bool,
    /// the mode's real pixel size (EnumDisplaySettings)
    pub w: i32,
    pub h: i32,
    pub num: i32,
}

/// Give each monitor its number: the main display 1, the others 2, 3 ... left to right.
pub fn number(mons: &mut [Mon]) {
    for m in mons.iter_mut() {
        m.num = if m.primary { 1 } else { 0 };
    }
    let mut n = 2;
    loop {
        let mut best: Option<usize> = None;
        for (j, m) in mons.iter().enumerate() {
            if m.num == 0 && best.is_none_or(|b| m.rc.left < mons[b].rc.left) {
                best = Some(j);
            }
        }
        let Some(b) = best else { break };
        mons[b].num = n;
        n += 1;
    }
}

/// Test monitors: "id,w,h,x,y,primary id2,..." (monitors.c `mons_set_fake`; work area = 40 px taskbar at the bottom).
pub fn fake(spec: &str) -> Vec<Mon> {
    let mut v = Vec::new();
    for (k, tok) in spec.split_whitespace().enumerate() {
        let mut parts = tok.split(',');
        let id = parts.next().unwrap_or("").to_string();
        let f: Vec<i32> = parts.map(|p| crate::ini::atoi(p) as i32).chain(std::iter::repeat(0)).take(5).collect();
        let rc = Rect { left: f[2], top: f[3], right: f[2] + f[0], bottom: f[3] + f[1] };
        let mut work = rc;
        work.bottom -= 40;
        v.push(Mon { dev: format!("\\\\.\\FAKE{}", k + 1), iface: id, rc, work, primary: f[4] != 0, w: f[0], h: f[1], num: 0 });
    }
    number(&mut v);
    v
}

pub fn primary(mons: &[Mon]) -> Option<usize> {
    mons.iter().position(|m| m.primary).or(if mons.is_empty() { None } else { Some(0) })
}

pub fn by_num(mons: &[Mon], num: i32) -> Option<usize> {
    mons.iter().position(|m| m.num == num)
}

fn seg(s: &str, k: usize) -> &str {
    s.split('#').nth(k).unwrap_or("")
}
fn uid(inst: &str) -> &str {
    inst.rsplit('&').next().unwrap_or(inst)
}

/// OBS display capture `monitor_id` -> our monitor: exact device path first, then same model + UID, then the same model
/// if only one such monitor is connected.
pub fn find_obs(mons: &[Mon], id: &str) -> Option<usize> {
    if id.is_empty() {
        return None;
    }
    if let Some(i) = mons.iter().position(|m| m.iface.eq_ignore_ascii_case(id)) {
        return Some(i);
    }
    let (model, inst) = (seg(id, 1), seg(id, 2));
    if model.is_empty() {
        return None;
    }
    if let Some(i) = mons.iter().position(|m| seg(&m.iface, 1).eq_ignore_ascii_case(model) && uid(seg(&m.iface, 2)).eq_ignore_ascii_case(uid(inst))) {
        return Some(i);
    }
    let hits: Vec<usize> = (0..mons.len()).filter(|&i| seg(&mons[i].iface, 1).eq_ignore_ascii_case(model)).collect();
    (hits.len() == 1).then(|| hits[0])
}

/// "ABC1234_UID1" from "\\?\DISPLAY#ABC1234#5&1a2b3c4d&0&UID1#{...}" (app.c `mon_token`, the key of ClipPing's
/// [LastScene] list).
pub fn token(m: &Mon) -> String {
    let (model, inst) = (seg(&m.iface, 1), seg(&m.iface, 2));
    if model.is_empty() {
        return m.dev.clone();
    }
    let mut s: String = model.chars().take(60).collect();
    if m.iface.matches('#').count() >= 2 {
        s.push('_');
        s.push_str(uid(inst));
    }
    s
}

/// Which monitor a "Show on" choice means (popups and the status icon); `clipped` = the monitor being clipped.
pub fn pick_monitor(mons: &[Mon], where_: i32, clipped: Option<usize>) -> Option<usize> {
    if where_ == W_NONE {
        return None;
    }
    let n = mons.len();
    if n <= 1 {
        return if n == 1 { Some(0) } else { None };
    }
    let p = primary(mons).unwrap_or(0);
    let clipped = clipped.filter(|&c| c < n);
    match where_ {
        W_SAME => return Some(clipped.unwrap_or(p)),
        W_MON1 => return Some(p),
        W_MON2 => return Some(by_num(mons, 2).unwrap_or(p)),
        _ => {}
    }
    let Some(c) = clipped else { return Some(p) };
    if p != c {
        return Some(p);
    }
    if let Some(i) = by_num(mons, 2).filter(|&i| i != c) {
        return Some(i);
    }
    (0..n).find(|&i| i != c).or(Some(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::W_OTHER;

    #[test]
    fn numbers_and_picks() {
        let m = fake("\\\\?\\DISPLAY#DEL1#5&a&0&UID1#{x},2560,1440,0,0,1 \\\\?\\DISPLAY#LG2#5&b&0&UID2#{x},1920,1080,-1920,0,0 \\\\?\\DISPLAY#LG3#5&c&0&UID3#{x},1920,1080,2560,0,0");
        assert_eq!(m.iter().map(|x| x.num).collect::<Vec<_>>(), vec![1, 2, 3]);
        assert_eq!(pick_monitor(&m, W_OTHER, Some(0)), Some(1));
        assert_eq!(pick_monitor(&m, W_OTHER, Some(1)), Some(0));
        assert_eq!(pick_monitor(&m, W_OTHER, None), Some(0));
        assert_eq!(pick_monitor(&m, W_SAME, Some(2)), Some(2));
        assert_eq!(pick_monitor(&m, W_MON2, None), Some(1));
        assert_eq!(pick_monitor(&m, W_NONE, None), None);
        assert_eq!(find_obs(&m, "\\\\?\\DISPLAY#LG2#5&zz&0&UID2#{x}"), Some(1));
        assert_eq!(find_obs(&m, "\\\\?\\DISPLAY#DEL1#7&q&0&UID9#{x}"), Some(0));
        assert_eq!(find_obs(&m, ""), None);
        assert_eq!(token(&m[1]), "LG2_UID2");
    }
}
