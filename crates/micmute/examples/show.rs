//! Prints the REAL Mic mute state, READ-ONLY: the mics, the default, each mic's mute flag, a change watch registered
//! and removed again, the sounds' sizes. Nothing is muted or unmuted, no sound
//! is played — the OS layers are the read-only ones (every change refused).
//!   cargo run -p bu-micmute --example micmute-show

use bu_micmute::real::{RealMicOs, RealSoundOut};
use bu_micmute::{sound, MicChoice, MicMute, MicOs, Sound, SoundSettings};
use std::sync::Arc;
use std::time::Instant;

fn main() {
    let os = Arc::new(RealMicOs::read_only());

    println!("== Microphones (active capture endpoints)");
    let t0 = Instant::now();
    match os.capture_devices() {
        Ok(list) => {
            for d in &list {
                let muted = os.is_muted(&d.id).map(|m| if m { "MUTED" } else { "live" }.to_string());
                println!(
                    "{} {:<48} {:<6} id={}",
                    if d.is_default { "*" } else { " " },
                    d.name,
                    muted.unwrap_or_else(|e| format!("({e})")),
                    d.id
                );
            }
            println!("({} mic(s), read in {} ms; * = Windows' default input device)", list.len(), t0.elapsed().as_millis());
        }
        Err(e) => println!("error: {e}"),
    }
    println!("default capture id: {:?}", os.default_capture());

    // the service on read-only layers: state works, every change is refused
    let mm = MicMute::new(os.clone(), Arc::new(RealSoundOut::new()));
    println!("\n== Service (choice = {:?})", MicChoice::Default);
    println!("state: {:?}", mm.state());
    println!("toggle on the read-only layer (must be refused): {:?}", mm.toggle().err());

    println!("\n== Change watch (registered, then removed — event-driven, nothing polls)");
    let t0 = Instant::now();
    let r = mm.start_watching(Arc::new(|s| println!("  change event: {s:?}")));
    println!("start_watching: {:?} in {} ms, watching = {}", r, t0.elapsed().as_millis(), mm.is_watching());
    mm.stop_watching();
    println!("stopped, watching = {}", mm.is_watching());

    println!("\n== Sounds (made in code; NOT played)");
    let s = SoundSettings::default();
    println!("defaults: {s:?}");
    for snd in Sound::ALL {
        let len = sound::wav(snd, s.volume).map(|w| w.len());
        println!("  {:<10} wav bytes at {} %: {:?}", snd.label(), s.volume, len);
    }
}
