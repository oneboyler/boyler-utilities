//! Read-only: which process owns the foreground window, and would the Win+H shortcut be sent from it? Sends no key, opens no
//! window, changes no setting. (Run from a terminal: the terminal is in front, so the answer is "not ours" - nothing sent.)

fn main() {
    #[cfg(windows)]
    {
        let fg = bu_voice::real::foreground_pid();
        println!("foreground window's process: {}", fg.map(|p| p.to_string()).unwrap_or_else(|| "none".into()));
        println!("is it this program's own window (Win+H would be sent): {}", bu_voice::real::foreground_is_ours());
    }
}
