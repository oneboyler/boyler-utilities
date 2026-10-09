//! Order 058's READ-ONLY proof on the real PC: what the keyboard picture would show right now. Prints the current keyboard
//! layout, the labels of the keys that differ between layouts, the remaps Windows has (the Scancode Map; none = none), and
//! whether a game / full-screen window or an administrator window is in front. Changes nothing, sends nothing, plays nothing.
//!
//! `cargo run -p bu-keysound --example keysound-show`

#[cfg(windows)]
fn main() {
    use bu_keysound::layout::{current, label, layout_id};
    use bu_keysound::remap::{self, name};
    let lay = current();
    println!("keyboard layout id: {} (0000041A Croatian, 0000081A Serbian Latin: both QWERTZ with Č Ć Ž Š Đ)", layout_id());
    for (code, what) in [(0x15u16, "the key where a US keyboard has Y"), (0x2C, "the key where a US keyboard has Z"), (0x27, "right of L"), (0x28, "next one"), (0x2B, "next one"), (0x1A, "right of P"), (0x1B, "next one"), (0x56, "right of left Shift"), (0x29, "left of 1")] {
        println!("  key {code:#06X} ({what}): {}", label(&lay, code));
    }
    match remap::real::read() {
        Ok(m) if m.is_empty() => println!("remaps Windows has: none"),
        Ok(m) => println!("remaps Windows has: {}", m.iter().map(|x| format!("{} -> {}", name(x.from), if x.to == 0 { "disabled".to_string() } else { name(x.to) })).collect::<Vec<_>>().join(", ")),
        Err(e) => println!("remaps Windows has: could not read ({e})"),
    }
    println!("a game / full-screen window in front: {}", bu_keysound::guard::game_in_front());
    println!("input blocked for macros right now: {:?}", bu_keysound::guard::input_blocked());
    // what the question costs (the key sounds ask it on a press, cached 1.5 s per front window)
    let t = std::time::Instant::now();
    for _ in 0..500 {
        std::hint::black_box(bu_keysound::guard::input_blocked());
    }
    println!("cost of one input_blocked() question: {:.1} us (measured, 500 calls)", t.elapsed().as_secs_f64() * 1e6 / 500.0);
}

#[cfg(not(windows))]
fn main() {}
