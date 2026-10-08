//! The pure part of the watcher (no Windows calls, so it is tested on its own): which processes were seen in the last
//! process snapshot, who listens for which exe names, and which exits are waited for without a process handle.
//!
//! Every snapshot replaces the seen list completely, so a process that ended drops out and a reused process id counts
//! as new again after any later snapshot. A process id whose exe name changed between two snapshots also counts as new
//! (the id was reused in between).

use std::collections::HashMap;

/// One subscriber: its id, its exe names (lower case, sorted) and its sink.
struct Sub<S> {
    id: u64,
    names: Vec<String>,
    sink: S,
}

/// One exit waited for by snapshots (the process refused even a SYNCHRONIZE handle).
struct Exit<E> {
    id: u64,
    pid: u32,
    /// exe name (lower case) the process had when the wait began: a different name under the same id = it ended.
    /// None = not known (the snapshot failed): then only a missing id counts as ended.
    name: Option<String>,
    on_exit: E,
}

/// What one snapshot found: the starts to report (sink, pid, exe file name as Windows lists it) and the exits.
pub struct Report<S, E> {
    pub starts: Vec<(S, u32, String)>,
    pub exits: Vec<E>,
}

/// The watcher's state. `S` = a start sink (cloned per report), `E` = an exit callback (given out once).
pub struct Watch<S, E> {
    /// pid -> exe name (lower case) from the last snapshot
    seen: HashMap<u32, String>,
    /// false until the first snapshot: that one only fills `seen` (processes already running are not starts)
    primed: bool,
    subs: Vec<Sub<S>>,
    exits: Vec<Exit<E>>,
    /// an exit wait was added since the last `take_exits_added` (the thread then snapshots once at once)
    exits_added: bool,
}

impl<S, E> Default for Watch<S, E> {
    fn default() -> Self {
        Self { seen: HashMap::new(), primed: false, subs: Vec::new(), exits: Vec::new(), exits_added: false }
    }
}

/// Lower case, sorted, no duplicates, no empty names.
pub fn clean_names(names: &[String]) -> Vec<String> {
    let mut v: Vec<String> = names.iter().map(|n| n.trim().to_ascii_lowercase()).filter(|n| !n.is_empty()).collect();
    v.sort();
    v.dedup();
    v
}

impl<S: Clone, E> Watch<S, E> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a subscriber (names are cleaned here).
    pub fn subscribe(&mut self, id: u64, names: &[String], sink: S) {
        self.subs.push(Sub { id, names: clean_names(names), sink });
    }

    /// Removes a subscriber and hands its sink back, so the caller drops it outside any lock.
    pub fn unsubscribe(&mut self, id: u64) -> Option<S> {
        let i = self.subs.iter().position(|s| s.id == id)?;
        Some(self.subs.remove(i).sink)
    }

    /// Waits for `pid` (exe name `name` now, if known) to end, checked at every snapshot.
    pub fn add_exit(&mut self, id: u64, pid: u32, name: Option<&str>, on_exit: E) {
        self.exits.push(Exit { id, pid, name: name.map(|n| n.to_ascii_lowercase()), on_exit });
        self.exits_added = true;
    }

    /// Cancels an exit wait and hands its callback back (None = it already fired or never existed).
    pub fn remove_exit(&mut self, id: u64) -> Option<E> {
        let i = self.exits.iter().position(|e| e.id == id)?;
        Some(self.exits.swap_remove(i).on_exit)
    }

    /// At least one subscriber listens for at least one name (the window-creation hook is needed).
    pub fn wants_starts(&self) -> bool {
        self.subs.iter().any(|s| !s.names.is_empty())
    }

    /// At least one exit is waited for by snapshots.
    pub fn has_exits(&self) -> bool {
        !self.exits.is_empty()
    }

    /// Nothing to do at all: the thread unhooks and ends.
    pub fn idle(&self) -> bool {
        !self.wants_starts() && !self.has_exits()
    }

    /// The process ids whose exit is waited for by snapshots.
    pub fn exit_pids(&self) -> Vec<u32> {
        self.exits.iter().map(|e| e.pid).collect()
    }

    /// True once if an exit wait was added since the last call.
    pub fn take_exits_added(&mut self) -> bool {
        std::mem::take(&mut self.exits_added)
    }

    /// Was this process id in the last snapshot? (Before the first snapshot nothing is seen.)
    pub fn is_seen(&self, pid: u32) -> bool {
        self.seen.contains_key(&pid)
    }

    /// The watcher went idle: forget the snapshot, so the next start primes again.
    pub fn forget(&mut self) {
        self.seen.clear();
        self.primed = false;
    }

    /// Applies a new snapshot (pid, exe file name). Returns every process new since the last snapshot whose name a
    /// subscriber listens for (none on the first snapshot), and every waited-for exit whose process is gone.
    pub fn apply(&mut self, procs: &[(u32, String)]) -> Report<S, E> {
        let mut starts = Vec::new();
        let mut next = HashMap::with_capacity(procs.len());
        for (pid, exe) in procs {
            let lower = exe.to_ascii_lowercase();
            if self.primed && self.seen.get(pid) != Some(&lower) {
                for s in &self.subs {
                    if s.names.binary_search(&lower).is_ok() {
                        starts.push((s.sink.clone(), *pid, exe.clone()));
                    }
                }
            }
            next.insert(*pid, lower);
        }
        self.seen = next;
        self.primed = true;
        let mut exits = Vec::new();
        let mut i = 0;
        while i < self.exits.len() {
            let e = &self.exits[i];
            let gone = match (&e.name, self.seen.get(&e.pid)) {
                (_, None) => true,
                (Some(was), Some(now)) => was != now,
                (None, Some(_)) => false,
            };
            if gone {
                exits.push(self.exits.swap_remove(i).on_exit);
            } else {
                i += 1;
            }
        }
        Report { starts, exits }
    }
}

/// The exe name of `pid` if it is in this snapshot (None = it is gone). Used before an exit wait starts.
pub fn alive_in(procs: &[(u32, String)], pid: u32) -> Option<&str> {
    procs.iter().find(|(p, _)| *p == pid).map(|(_, n)| n.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(v: &[(u32, &str)]) -> Vec<(u32, String)> {
        v.iter().map(|(p, n)| (*p, n.to_string())).collect()
    }

    fn starts<E>(r: &Report<&'static str, E>) -> Vec<(&'static str, u32, String)> {
        let mut v: Vec<_> = r.starts.iter().map(|(s, p, n)| (*s, *p, n.clone())).collect();
        v.sort();
        v
    }

    #[test]
    fn first_snapshot_only_primes() {
        let mut w: Watch<&str, ()> = Watch::new();
        w.subscribe(1, &["Game.exe".into()], "display");
        let r = w.apply(&snap(&[(10, "game.exe"), (11, "explorer.exe")]));
        assert!(r.starts.is_empty(), "already running is not a start");
        assert!(w.is_seen(10) && w.is_seen(11) && !w.is_seen(12));
    }

    #[test]
    fn new_pids_are_reported_to_every_subscriber_that_names_them() {
        let mut w: Watch<&str, ()> = Watch::new();
        w.subscribe(1, &["GAME.EXE".into(), "other.exe".into()], "display");
        w.subscribe(2, &["game.exe".into()], "mouse");
        w.subscribe(3, &["nothing.exe".into()], "none");
        w.apply(&snap(&[(10, "explorer.exe")]));
        // the game (no window yet) and its launcher's helper both new: the game is reported even though the
        // snapshot was triggered by another process's window
        let r = w.apply(&snap(&[(10, "explorer.exe"), (20, "Game.exe"), (21, "helper.exe")]));
        assert_eq!(starts(&r), vec![("display", 20, "Game.exe".to_string()), ("mouse", 20, "Game.exe".to_string())]);
        // the same process in the next snapshot is not new any more
        let r = w.apply(&snap(&[(10, "explorer.exe"), (20, "Game.exe")]));
        assert!(r.starts.is_empty());
    }

    #[test]
    fn a_reused_pid_is_new_again_after_a_snapshot() {
        let mut w: Watch<&str, ()> = Watch::new();
        w.subscribe(1, &["game.exe".into()], "s");
        w.apply(&snap(&[(30, "game.exe")]));
        w.apply(&snap(&[])); // it ended: dropped from the seen list
        assert!(!w.is_seen(30));
        let r = w.apply(&snap(&[(30, "game.exe")]));
        assert_eq!(starts(&r), vec![("s", 30, "game.exe".to_string())]);
        // reused between two snapshots by a different exe: still new (the name changed)
        let r = w.apply(&snap(&[(30, "tool.exe")]));
        assert!(r.starts.is_empty());
        let r = w.apply(&snap(&[(30, "game.exe")]));
        assert_eq!(r.starts.len(), 1);
    }

    #[test]
    fn unsubscribe_stops_reports_and_idles() {
        let mut w: Watch<&str, ()> = Watch::new();
        assert!(w.idle());
        w.subscribe(1, &["game.exe".into()], "a");
        w.subscribe(2, &[], "empty");
        assert!(w.wants_starts() && !w.idle());
        w.apply(&snap(&[]));
        assert_eq!(w.unsubscribe(1), Some("a"));
        assert_eq!(w.unsubscribe(1), None);
        let r = w.apply(&snap(&[(40, "game.exe")]));
        assert!(r.starts.is_empty());
        assert!(!w.wants_starts(), "a subscriber without names does not need the hook");
        assert!(w.idle());
        w.forget();
        assert!(!w.is_seen(40));
        // after forgetting, the next snapshot primes again
        w.subscribe(3, &["game.exe".into()], "b");
        assert!(w.apply(&snap(&[(40, "game.exe")])).starts.is_empty());
    }

    #[test]
    fn exit_fallback_fires_when_the_pid_is_gone_or_reused() {
        let mut w: Watch<&str, u32> = Watch::new();
        w.apply(&snap(&[(50, "game.exe"), (51, "game2.exe")]));
        w.add_exit(1, 50, Some("Game.exe"), 500);
        w.add_exit(2, 51, Some("game2.exe"), 510);
        w.add_exit(3, 52, Some("x.exe"), 520);
        w.add_exit(4, 51, None, 540);
        assert!(w.take_exits_added() && !w.take_exits_added());
        assert!(w.has_exits() && !w.idle());
        let mut pids = w.exit_pids();
        pids.sort();
        assert_eq!(pids, vec![50, 51, 51, 52]);
        assert_eq!(w.remove_exit(3), Some(520));
        let r = w.apply(&snap(&[(50, "game.exe"), (51, "game2.exe")]));
        assert!(r.exits.is_empty(), "both still running");
        // 50 ended; 51's id now belongs to another exe (it ended and the id was reused)
        let mut r = w.apply(&snap(&[(51, "notepad.exe")])).exits;
        r.sort();
        assert_eq!(r, vec![500, 510]);
        // the wait whose exe name was not known ends only when the id is missing
        assert!(w.has_exits());
        assert_eq!(w.apply(&snap(&[])).exits, vec![540]);
        assert!(!w.has_exits());
        assert_eq!(w.remove_exit(1), None, "fired once, then gone");
    }

    #[test]
    fn alive_in_finds_the_pid() {
        let s = snap(&[(7, "Game.exe")]);
        assert_eq!(alive_in(&s, 7), Some("Game.exe"));
        assert_eq!(alive_in(&s, 8), None);
    }

    #[test]
    fn names_are_cleaned() {
        assert_eq!(clean_names(&["B.exe".into(), "a.EXE".into(), "b.exe".into(), " ".into()]), vec!["a.exe".to_string(), "b.exe".to_string()]);
    }
}
