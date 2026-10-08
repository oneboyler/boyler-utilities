//! Read-only: the dictation languages Windows offers on this PC (what the Voice to text language picker offers), the
//! switches that could stop it, and whether the online dictation recognizer can be created and its dictation constraint
//! compiled. Never starts listening, never records, changes no setting.

fn main() {
    #[cfg(windows)]
    {
        use bu_voice::Engine;
        let eng = bu_voice::real::WinSpeech::new();
        let t0 = std::time::Instant::now();
        let r = eng.languages();
        println!("read in {:.1} ms (what opening the Voice to text tab costs)", t0.elapsed().as_secs_f64() * 1000.0);
        println!("Windows' speech language: {}", bu_voice::real::system_language().unwrap_or_else(|| "(could not read)".into()));
        let langs = match r {
            Ok(l) if l.is_empty() => {
                println!("dictation languages: none");
                l
            }
            Ok(l) => {
                println!("dictation languages Windows offers on this PC ({}):", l.len());
                for x in &l {
                    println!("  {:<8} {}", x.tag, x.name);
                }
                l
            }
            Err(e) => {
                println!("dictation languages: could not read ({e})");
                Vec::new()
            }
        };
        let (online, mic) = bu_voice::real::switches();
        println!(
            "Online speech recognition switch (HKCU HasAccepted): {}",
            match online {
                Some(1) => "on".to_string(),
                Some(0) => "off".to_string(),
                Some(v) => format!("{v}"),
                None => "not set (never chosen)".to_string(),
            }
        );
        for (who, v) in mic {
            println!("microphone access ({who}): {v}");
        }
        let t0 = std::time::Instant::now();
        let c = eng.check();
        println!("check (read-only, {:.1} ms): {}", t0.elapsed().as_secs_f64() * 1000.0, c.map(|_| "ready".to_string()).unwrap_or_else(|e| e.to_string()));
        if let Some(first) = langs.first() {
            let t0 = std::time::Instant::now();
            let p = bu_voice::real::prepare(first);
            let ms = t0.elapsed().as_secs_f64() * 1000.0;
            match p {
                Ok(tag) => println!("recognizer created + dictation constraint compiled: Success, language {tag} ({ms:.0} ms; nothing listened)"),
                Err(e) => println!("recognizer / dictation constraint for {}: {e} ({ms:.0} ms; nothing listened)", first.tag),
            }
        }
    }
}
