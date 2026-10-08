//! Row 2 — **Repair Windows files** (admin): `DISM /Online /Cleanup-Image /RestoreHealth`, then `sfc /scannow`, with
//! progress from their output, a Cancel, and a result line. Keeps running when the menu closes (the app holds the
//! [`RepairRun`]; it lives on its own thread that blocks on the programs' output — no polling).
//!
//! Output facts (research, see the order notes): sfc writes **UTF-16LE** to a pipe (no BOM promised) with
//! "Verification 34% complete."; DISM writes a bar like `[====   18.3%   ]` in the console code page. Lines may end in
//! `\r` (rewritten progress) or `\r\n`. The result words are English; on another Windows language the outcome is
//! read from exit codes where possible and otherwise shown as the program's own last line (**unclear**).

use crate::os::{FixOs, LocalTime, ProcCtl, Spawned};
use crate::{FixError, Result};
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

// ---------------------------------------------------------------- output decoding

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Enc {
    Unknown,
    Utf16,
    Bytes,
}

/// Turns a program's raw output into lines, whichever encoding it uses (UTF-16LE detected by its BOM or by zero
/// bytes in the odd positions — what ASCII text in UTF-16LE looks like). Lines split on `\r` and `\n`; empty lines
/// are dropped.
pub struct LineDecoder {
    enc: Enc,
    carry: Vec<u8>,
    line: String,
}

impl Default for LineDecoder {
    fn default() -> Self {
        LineDecoder { enc: Enc::Unknown, carry: Vec::new(), line: String::new() }
    }
}

impl LineDecoder {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.carry.extend_from_slice(bytes);
        if self.enc == Enc::Unknown {
            if self.carry.len() < 2 {
                return Vec::new();
            }
            self.enc = if self.carry.starts_with(&[0xFF, 0xFE]) {
                self.carry.drain(..2);
                Enc::Utf16
            } else {
                let sample = &self.carry[..self.carry.len().min(64) & !1];
                let odd = sample.len() / 2;
                let zeros = sample.iter().skip(1).step_by(2).filter(|&&b| b == 0).count();
                if odd > 0 && zeros * 2 >= odd { Enc::Utf16 } else { Enc::Bytes }
            };
        }
        let text = match self.enc {
            Enc::Utf16 => {
                let usable = self.carry.len() & !1;
                let units: Vec<u16> =
                    self.carry[..usable].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                self.carry.drain(..usable);
                String::from_utf16_lossy(&units)
            }
            _ => {
                // keep an incomplete UTF-8 sequence for the next chunk
                let cut = match std::str::from_utf8(&self.carry) {
                    Ok(_) => self.carry.len(),
                    Err(e) if e.error_len().is_none() => e.valid_up_to(),
                    Err(_) => self.carry.len(),
                };
                let s = String::from_utf8_lossy(&self.carry[..cut]).into_owned();
                self.carry.drain(..cut);
                s
            }
        };
        let mut out = Vec::new();
        for ch in text.chars() {
            if ch == '\r' || ch == '\n' {
                let l = std::mem::take(&mut self.line);
                if !l.trim().is_empty() {
                    out.push(l.trim_end().to_string());
                }
            } else if ch != '\0' && ch != '\u{feff}' {
                self.line.push(ch);
            }
        }
        out
    }

    /// The last unfinished line (at the end of the output).
    pub fn finish(&mut self) -> Option<String> {
        let rest = std::mem::take(&mut self.carry);
        let mut lines = self.push(&rest);
        let l = std::mem::take(&mut self.line);
        if !l.trim().is_empty() {
            lines.push(l.trim_end().to_string());
        }
        lines.pop()
    }
}

/// Decodes a whole output at once.
pub fn decode_all(bytes: &[u8]) -> String {
    let mut d = LineDecoder::default();
    let mut lines = d.push(bytes);
    lines.extend(d.finish());
    lines.join("\n")
}

/// The last percentage in a line: "Verification 34% complete." → 34, "[====  18.3%  ]" → 18.3, "18,3 %" → 18.3.
pub fn percent(line: &str) -> Option<f32> {
    let chars: Vec<char> = line.chars().collect();
    let mut found = None;
    for (i, &c) in chars.iter().enumerate() {
        if c != '%' {
            continue;
        }
        let mut end = i;
        if end > 0 && chars[end - 1] == ' ' {
            end -= 1;
        }
        let mut start = end;
        while start > 0 && (chars[start - 1].is_ascii_digit() || chars[start - 1] == '.' || chars[start - 1] == ',') {
            start -= 1;
        }
        let num: String = chars[start..end].iter().map(|&c| if c == ',' { '.' } else { c }).collect();
        if let Ok(v) = num.trim_matches('.').parse::<f32>() {
            if (0.0..=100.0).contains(&v) {
                found = Some(v);
            }
        }
    }
    found
}

// ---------------------------------------------------------------- results

/// Which program is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Dism,
    Sfc,
}

/// What the row shows while running.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Progress {
    pub phase: Phase,
    /// 0–100, `None` until the program prints its first percentage.
    pub percent: Option<f32>,
}

impl Progress {
    /// DESIGN's row text: "DISM · checking the Windows image · 34 %", "sfc · checking system files · 71 %".
    pub fn text(&self) -> String {
        let what = match self.phase {
            Phase::Dism => "DISM · checking the Windows image",
            Phase::Sfc => "sfc · checking system files",
        };
        match self.percent {
            Some(p) => format!("{what} · {} %", p.floor() as u32),
            None => what.to_string(),
        }
    }
}

/// How DISM ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DismResult {
    /// "The restore operation completed successfully." — nothing needed fixing.
    Healthy,
    /// "The component store corruption was repaired."
    Repaired,
    /// Exit 740 / "Elevated permissions are required to run DISM".
    NotAdmin,
    /// Anything else (e.g. 0x800F081F "The source files could not be found." — no internet / no source).
    Failed { exit_code: u32, line: String },
    Cancelled,
}

/// Reads DISM's end from its exit code + output.
pub fn classify_dism(exit_code: u32, text: &str) -> DismResult {
    let low = text.to_ascii_lowercase();
    if exit_code == 740 || low.contains("elevated permissions are required") {
        return DismResult::NotAdmin;
    }
    if exit_code == 0 {
        return if low.contains("corruption was repaired") { DismResult::Repaired } else { DismResult::Healthy };
    }
    let line = text
        .lines()
        .rev()
        .find(|l| l.trim_start().to_ascii_lowercase().starts_with("error"))
        .or_else(|| text.lines().rev().find(|l| !l.trim().is_empty() && percent(l).is_none()))
        .unwrap_or("")
        .trim()
        .to_string();
    DismResult::Failed { exit_code, line }
}

/// How sfc ended (its English result lines, research §3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SfcResult {
    /// "Windows Resource Protection did not find any integrity violations."
    NoViolations,
    /// "… found corrupt files and successfully repaired them."
    Repaired,
    /// "… found corrupt files but was unable to fix some of them."
    NotAllFixed,
    /// "… could not perform the requested operation."
    CouldNotRun,
    /// "There is a system repair pending which requires reboot to complete."
    RepairPending,
    /// "You must be an administrator running a console session in order to use the sfc utility."
    NotAdmin,
    /// No known sentence (another Windows language): the program's last line.
    Unknown(String),
    Cancelled,
}

pub fn classify_sfc(text: &str) -> SfcResult {
    let low = text.to_ascii_lowercase();
    if low.contains("did not find any integrity violations") {
        SfcResult::NoViolations
    } else if low.contains("found corrupt files and successfully repaired them") {
        SfcResult::Repaired
    } else if low.contains("unable to fix some of them") {
        SfcResult::NotAllFixed
    } else if low.contains("repair pending which requires reboot") {
        SfcResult::RepairPending
    } else if low.contains("could not perform the requested operation") {
        SfcResult::CouldNotRun
    } else if low.contains("must be an administrator") {
        SfcResult::NotAdmin
    } else {
        let last = text.lines().rev().find(|l| !l.trim().is_empty() && percent(l).is_none()).unwrap_or("");
        SfcResult::Unknown(last.trim().to_string())
    }
}

/// Counts sfc's repaired files in CBS.log: lines `… [SR] Repairing corrupted file …` stamped at or after `since`
/// (CBS.log lines start "YYYY-MM-DD HH:MM:SS, …", local time — **guess**: the log's clock is local). One line per
/// file per the Microsoft log guide's examples.
pub fn count_sr_repairs(cbs: &str, since: LocalTime) -> u32 {
    let from = format!("{:04}-{:02}-{:02} {:02}:{:02}", since.year, since.month, since.day, since.hour, since.minute);
    cbs.lines()
        .filter(|l| l.len() >= 16 && l.is_char_boundary(16) && l[..16] >= *from.as_str())
        .filter(|l| l.contains("[SR] Repairing corrupted file"))
        .count() as u32
}

/// The result line of the whole repair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepairOutcome {
    /// "✓ No problems found".
    NoProblems,
    /// "Repaired N files · restart to finish" (`files` = from CBS.log; `None` when it couldn't be counted).
    Repaired { files: Option<u32> },
    /// sfc found damage it couldn't fix.
    NotAllFixed,
    /// A repair from earlier waits for a restart; sfc refuses to run until then.
    RestartFirst,
    NeedsAdmin,
    /// A program failed (`detail` = its own words).
    Failed { phase: Phase, detail: String },
    /// Finished, but the result sentence was not one we know (another Windows language): sfc's last line.
    Finished { last_line: String },
    Cancelled,
}

impl RepairOutcome {
    /// The row's end text. DESIGN wording for "No problems" and "Repaired"; the other wordings are calls (not drawn).
    pub fn line(&self) -> String {
        match self {
            RepairOutcome::NoProblems => "✓ No problems found".into(),
            RepairOutcome::Repaired { files: Some(1) } => "Repaired 1 file · restart to finish".into(),
            RepairOutcome::Repaired { files: Some(n) } if *n > 1 => format!("Repaired {n} files · restart to finish"),
            RepairOutcome::Repaired { .. } => "Repaired Windows files · restart to finish".into(),
            RepairOutcome::NotAllFixed => "Some files couldn't be repaired".into(),
            RepairOutcome::RestartFirst => "Restart Windows first, then run it again".into(),
            RepairOutcome::NeedsAdmin => "Needs admin".into(),
            RepairOutcome::Failed { phase: Phase::Dism, detail } => format!("DISM failed · {detail}"),
            RepairOutcome::Failed { phase: Phase::Sfc, detail } => format!("sfc failed · {detail}"),
            RepairOutcome::Finished { last_line } => last_line.clone(),
            RepairOutcome::Cancelled => "Cancelled".into(),
        }
    }
}

/// Everything about one run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepairReport {
    pub dism: DismResult,
    /// `None` = sfc never started (cancelled during DISM).
    pub sfc: Option<SfcResult>,
    pub outcome: RepairOutcome,
    pub started: LocalTime,
    pub finished: LocalTime,
}

impl RepairReport {
    /// The row's end text with the finish time where DESIGN shows one: "✓ No problems found · today, 21:37".
    pub fn line(&self, now: LocalTime) -> String {
        match self.outcome {
            RepairOutcome::NoProblems => {
                format!("{} · {}", self.outcome.line(), crate::restore::format_relative(self.finished, now))
            }
            _ => self.outcome.line(),
        }
    }
}

/// Puts DISM's and sfc's results together. sfc runs even when DISM failed (DISM fixes Windows' own repair source —
/// without internet it fails, but sfc can still repair from what is there).
pub fn combine(dism: &DismResult, sfc: Option<&SfcResult>, repaired_files: Option<u32>) -> RepairOutcome {
    if *dism == DismResult::Cancelled || matches!(sfc, Some(SfcResult::Cancelled)) {
        return RepairOutcome::Cancelled;
    }
    if *dism == DismResult::NotAdmin || matches!(sfc, Some(SfcResult::NotAdmin)) {
        return RepairOutcome::NeedsAdmin;
    }
    match sfc {
        None => RepairOutcome::Cancelled,
        Some(SfcResult::NoViolations) => match dism {
            DismResult::Repaired => RepairOutcome::Repaired { files: None },
            DismResult::Failed { exit_code, line } => {
                RepairOutcome::Failed { phase: Phase::Dism, detail: dism_detail(*exit_code, line) }
            }
            _ => RepairOutcome::NoProblems,
        },
        Some(SfcResult::Repaired) => RepairOutcome::Repaired { files: repaired_files.filter(|&n| n > 0) },
        Some(SfcResult::NotAllFixed) => RepairOutcome::NotAllFixed,
        Some(SfcResult::RepairPending) => RepairOutcome::RestartFirst,
        Some(SfcResult::CouldNotRun) => {
            RepairOutcome::Failed { phase: Phase::Sfc, detail: "Windows Resource Protection could not run".into() }
        }
        Some(SfcResult::Unknown(l)) => RepairOutcome::Finished { last_line: l.clone() },
        Some(SfcResult::NotAdmin) => RepairOutcome::NeedsAdmin,
        Some(SfcResult::Cancelled) => RepairOutcome::Cancelled,
    }
}

fn dism_detail(code: u32, line: &str) -> String {
    if code == 0x800F_081F || line.contains("0x800f081f") {
        "Windows couldn't download the repair files · check the internet".into()
    } else if line.is_empty() {
        format!("error 0x{code:08X}")
    } else {
        line.to_string()
    }
}

// ---------------------------------------------------------------- the run

struct Shared {
    cancelled: AtomicBool,
    ctl: Mutex<Option<Arc<dyn ProcCtl>>>,
    progress: Mutex<Progress>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A running repair. Drop it and the repair keeps going to its end (the thread is detached) — call [`RepairRun::cancel`]
/// to stop it.
pub struct RepairRun {
    shared: Arc<Shared>,
    handle: Option<JoinHandle<RepairReport>>,
}

impl RepairRun {
    /// Starts DISM then sfc. Needs admin (both programs refuse without it). `on_progress` is called from the run's
    /// thread whenever the phase or the whole percent changes.
    pub fn start(os: Arc<dyn FixOs>, on_progress: impl Fn(Progress) + Send + 'static) -> Result<RepairRun> {
        if !os.is_elevated() {
            return Err(FixError::NeedsAdmin("DISM and sfc".into()));
        }
        let shared = Arc::new(Shared {
            cancelled: AtomicBool::new(false),
            ctl: Mutex::new(None),
            progress: Mutex::new(Progress { phase: Phase::Dism, percent: None }),
        });
        let sh = shared.clone();
        let handle = std::thread::Builder::new()
            .name("bu-quickfix-repair".into())
            .spawn(move || run(os.as_ref(), &sh, &on_progress))
            .map_err(|e| FixError::Os { context: format!("repair thread: {e}"), code: 0 })?;
        Ok(RepairRun { shared, handle: Some(handle) })
    }

    /// Stops the program that is running now (and everything it started); sfc is not started after a cancelled DISM.
    pub fn cancel(&self) {
        self.shared.cancelled.store(true, Ordering::SeqCst);
        if let Some(c) = lock(&self.shared.ctl).clone() {
            c.kill();
        }
    }

    pub fn progress(&self) -> Progress {
        *lock(&self.shared.progress)
    }

    pub fn is_finished(&self) -> bool {
        self.handle.as_ref().is_none_or(|h| h.is_finished())
    }

    /// Waits for the end.
    pub fn wait(mut self) -> RepairReport {
        let h = self.handle.take();
        match h.map(|h| h.join()) {
            Some(Ok(r)) => r,
            _ => {
                let t = LocalTime { year: 0, month: 0, day: 0, hour: 0, minute: 0 };
                RepairReport {
                    dism: DismResult::Cancelled,
                    sfc: None,
                    outcome: RepairOutcome::Failed { phase: Phase::Dism, detail: "the repair thread stopped".into() },
                    started: t,
                    finished: t,
                }
            }
        }
    }
}

/// Runs one program to its end: progress while reading, then its exit code + whole text. `None` = cancelled.
fn run_one(
    os: &dyn FixOs,
    sh: &Shared,
    phase: Phase,
    program: &str,
    args: &[&str],
    on_progress: &dyn Fn(Progress),
) -> std::result::Result<Option<(u32, String)>, String> {
    if sh.cancelled.load(Ordering::SeqCst) {
        return Ok(None);
    }
    let set = |p: Progress| {
        let changed = {
            let mut cur = lock(&sh.progress);
            let same = cur.phase == p.phase && cur.percent.map(|v| v.floor()) == p.percent.map(|v| v.floor());
            *cur = p;
            !same
        };
        if changed {
            on_progress(p);
        }
    };
    // a new phase always reaches the app
    *lock(&sh.progress) = Progress { phase, percent: None };
    on_progress(Progress { phase, percent: None });
    let Spawned { mut output, ctl } = os.spawn(program, args).map_err(|e| e.to_string())?;
    *lock(&sh.ctl) = Some(ctl.clone());
    if sh.cancelled.load(Ordering::SeqCst) {
        ctl.kill(); // cancel came between the start and here
    }
    let mut dec = LineDecoder::default();
    let mut lines: Vec<String> = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match output.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                for l in dec.push(&buf[..n]) {
                    if let Some(p) = percent(&l) {
                        set(Progress { phase, percent: Some(p) });
                    }
                    lines.push(l);
                }
            }
        }
    }
    lines.extend(dec.finish());
    let code = ctl.wait().map_err(|e| e.to_string())?;
    *lock(&sh.ctl) = None;
    if sh.cancelled.load(Ordering::SeqCst) {
        return Ok(None);
    }
    // progress lines repeat thousands of times; keep the words
    let text: Vec<&str> = lines.iter().map(String::as_str).filter(|l| percent(l).is_none() || l.contains("complete")).collect();
    Ok(Some((code, text.join("\n"))))
}

fn run(os: &dyn FixOs, sh: &Shared, on_progress: &dyn Fn(Progress)) -> RepairReport {
    let started = os.local(os.now());
    let report = |dism: DismResult, sfc: Option<SfcResult>, outcome: RepairOutcome| RepairReport {
        dism,
        sfc,
        outcome,
        started,
        finished: os.local(os.now()),
    };
    let dism = match run_one(os, sh, Phase::Dism, "dism.exe", &["/Online", "/Cleanup-Image", "/RestoreHealth"], on_progress) {
        Ok(Some((code, text))) => classify_dism(code, &text),
        Ok(None) => DismResult::Cancelled,
        Err(e) => {
            return report(
                DismResult::Failed { exit_code: 0, line: e.clone() },
                None,
                RepairOutcome::Failed { phase: Phase::Dism, detail: e },
            )
        }
    };
    if dism == DismResult::Cancelled || dism == DismResult::NotAdmin {
        let outcome = combine(&dism, None, None);
        return report(dism, None, outcome);
    }
    let sfc = match run_one(os, sh, Phase::Sfc, "sfc.exe", &["/scannow"], on_progress) {
        Ok(Some((_code, text))) => classify_sfc(&text),
        Ok(None) => SfcResult::Cancelled,
        Err(e) => return report(dism, None, RepairOutcome::Failed { phase: Phase::Sfc, detail: e }),
    };
    let files = if sfc == SfcResult::Repaired { os.cbs_log_tail().ok().map(|t| count_sr_repairs(&t, started)) } else { None };
    let outcome = combine(&dism, Some(&sfc), files);
    report(dism, Some(sfc), outcome)
}

