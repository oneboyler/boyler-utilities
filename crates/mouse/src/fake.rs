//! The FAKE OS layer for tests: an in-memory PC. Every write is recorded in `log` so tests can check exactly what
//! would have gone to Windows / the mouse / Raw Accel.

use crate::error::{Error, Result};
use crate::os::*;
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};

/// A fake mouse answering vendor requests: `answer(request) -> reply`. Tests plug in a protocol model.
pub type FakeMouseFn = Box<dyn FnMut(&[u8]) -> Result<Vec<u8>>>;

pub struct FakeOs {
    pub win: BTreeMap<WinSetting, WinRaw>,
    pub reg: BTreeMap<(Hive, String, String), RegValue>,
    pub elevated: bool,
    pub hid: Vec<HidInfo>,
    /// per interface path: the fake mouse
    pub mice: BTreeMap<String, FakeMouseFn>,
    pub rawaccel_version: Option<DriverVersion>,
    pub files: BTreeMap<PathBuf, String>,
    /// the app's own files (`read_bytes` / `write_bytes`): in memory, never on the disk
    pub byte_files: BTreeMap<PathBuf, Vec<u8>>,
    /// what the fake driver holds (the bytes of the last WRITE; a READ returns them)
    pub rawaccel_driver: Vec<u8>,
    /// how many WRITE ioctls were made
    pub rawaccel_byte_writes: u32,
    /// what Raw Accel's writer.exe was given (settings file path, JSON), in order
    pub rawaccel_writes: Vec<(PathBuf, String)>,
    /// a writer error to return once (e.g. its validation message)
    pub rawaccel_refuse: VecDeque<String>,
    /// every change, in order ("win_set PointerSpeed Num(12)", "reg_write …", "reload_cursors", …)
    pub log: Vec<String>,
    /// make the next N writes of any kind fail with "access denied"
    pub deny_writes: u32,
    pub env: BTreeMap<String, String>,
}

impl Default for FakeOs {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeOs {
    /// A PC with Windows' default mouse settings and the Windows Aero cursors (as measured on Windows 11).
    pub fn new() -> Self {
        let mut f = FakeOs {
            win: BTreeMap::new(),
            reg: BTreeMap::new(),
            elevated: false,
            hid: Vec::new(),
            mice: BTreeMap::new(),
            rawaccel_version: None,
            files: BTreeMap::new(),
            byte_files: BTreeMap::new(),
            rawaccel_driver: Vec::new(),
            rawaccel_byte_writes: 0,
            rawaccel_writes: Vec::new(),
            rawaccel_refuse: VecDeque::new(),
            log: Vec::new(),
            deny_writes: 0,
            env: BTreeMap::new(),
        };
        f.win.insert(WinSetting::PointerSpeed, WinRaw::Num(10));
        f.win.insert(WinSetting::Precision, WinRaw::Mouse([6, 10, 1]));
        f.win.insert(WinSetting::ScrollLines, WinRaw::Num(3));
        f.win.insert(WinSetting::DoubleClick, WinRaw::Num(500));
        f.win.insert(WinSetting::SwapButtons, WinRaw::Bool(false));
        f.env.insert("systemroot".into(), r"C:\Windows".into());
        let aero = [
            ("Arrow", r"C:\Windows\cursors\aero_arrow.cur"),
            ("Help", r"C:\Windows\cursors\aero_helpsel.cur"),
            ("AppStarting", r"C:\Windows\cursors\aero_working.ani"),
            ("Wait", r"C:\Windows\cursors\aero_busy.ani"),
            ("Crosshair", ""),
            ("IBeam", ""),
            ("NWPen", r"C:\Windows\cursors\aero_pen.cur"),
            ("No", r"C:\Windows\cursors\aero_unavail.cur"),
            ("SizeNS", r"C:\Windows\cursors\aero_ns.cur"),
            ("SizeWE", r"C:\Windows\cursors\aero_ew.cur"),
            ("SizeNWSE", r"C:\Windows\cursors\aero_nwse.cur"),
            ("SizeNESW", r"C:\Windows\cursors\aero_nesw.cur"),
            ("SizeAll", r"C:\Windows\cursors\aero_move.cur"),
            ("UpArrow", r"C:\Windows\cursors\aero_up.cur"),
            ("Hand", r"C:\Windows\cursors\aero_link.cur"),
            ("Pin", r"C:\Windows\cursors\aero_pin.cur"),
            ("Person", r"C:\Windows\cursors\aero_person.cur"),
        ];
        let cur = r"Control Panel\Cursors";
        let def = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Control Panel\Cursors\Default";
        for (n, p) in aero {
            f.reg.insert((Hive::Hkcu, cur.into(), n.into()), RegValue::ExpandSz(p.into()));
            f.reg.insert((Hive::Hklm, def.into(), n.into()), RegValue::ExpandSz(p.into()));
        }
        f.reg.insert((Hive::Hkcu, cur.into(), "".into()), RegValue::Sz("Windows Default".into()));
        f.reg.insert((Hive::Hkcu, cur.into(), "Scheme Source".into()), RegValue::Dword(2));
        f.reg.insert((Hive::Hkcu, cur.into(), "CursorBaseSize".into()), RegValue::Dword(32));
        f.reg.insert((Hive::Hkcu, r"Software\Microsoft\Accessibility".into(), "CursorSize".into()), RegValue::Dword(1));
        let aero_scheme: Vec<&str> = aero.iter().map(|(_, p)| *p).collect();
        f.reg.insert(
            (Hive::Hklm, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Control Panel\Cursors\Schemes".into(), "Windows Aero".into()),
            RegValue::ExpandSz(aero_scheme.join(",") + ",@main.cpl,-1020"),
        );
        let black: Vec<String> = aero.iter().map(|(n, _)| if *n == "Pin" || *n == "Person" { String::new() } else { format!(r"%SystemRoot%\cursors\{}_r.cur", n.to_lowercase()) }).collect();
        f.reg.insert(
            (Hive::Hklm, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Control Panel\Cursors\Schemes".into(), "Windows Black".into()),
            RegValue::ExpandSz(black.join(",")),
        );
        f
    }

    fn write_gate(&mut self, what: &str) -> Result<()> {
        if self.deny_writes > 0 {
            self.deny_writes -= 1;
            return Err(Error::NeedsAdmin { what: what.into() });
        }
        Ok(())
    }

    pub fn reg_get(&self, hive: Hive, path: &str, name: &str) -> Option<&RegValue> {
        self.reg.get(&(hive, path.to_string(), name.to_string()))
    }
}

impl MouseOs for FakeOs {
    fn win_get(&self, s: WinSetting) -> Result<WinRaw> {
        self.win.get(&s).copied().ok_or_else(|| Error::os(format!("fake win_get {s:?}"), 1))
    }

    fn win_set(&mut self, s: WinSetting, v: WinRaw) -> Result<()> {
        self.write_gate(&format!("{s:?}"))?;
        self.log.push(format!("win_set {s:?} {v:?}"));
        self.win.insert(s, v);
        Ok(())
    }

    fn reg_read(&self, hive: Hive, path: &str, name: &str) -> Result<Option<RegValue>> {
        Ok(self.reg_get(hive, path, name).cloned())
    }

    fn reg_write(&mut self, path: &str, name: &str, value: &RegValue) -> Result<()> {
        self.write_gate(path)?;
        self.log.push(format!("reg_write HKCU\\{path}\\{name} = {value:?}"));
        self.reg.insert((Hive::Hkcu, path.into(), name.into()), value.clone());
        Ok(())
    }

    fn reg_values(&self, hive: Hive, path: &str) -> Result<Vec<(String, RegValue)>> {
        Ok(self.reg.iter().filter(|((h, p, _), _)| *h == hive && p.eq_ignore_ascii_case(path)).map(|((_, _, n), v)| (n.clone(), v.clone())).collect())
    }

    fn reload_cursors(&mut self) -> Result<()> {
        self.log.push("reload_cursors".into());
        Ok(())
    }

    fn set_system_cursor(&mut self, file: &str, ocr_id: u32) -> Result<()> {
        self.log.push(format!("set_system_cursor {ocr_id} {file}"));
        Ok(())
    }

    fn expand_env(&self, s: &str) -> String {
        let mut out = String::new();
        let mut rest = s;
        while let Some(i) = rest.find('%') {
            out.push_str(&rest[..i]);
            let after = &rest[i + 1..];
            match after.find('%') {
                Some(j) => {
                    let var = &after[..j];
                    match self.env.get(&var.to_ascii_lowercase()) {
                        Some(v) => out.push_str(v),
                        None => {
                            out.push('%');
                            out.push_str(var);
                            out.push('%');
                        }
                    }
                    rest = &after[j + 1..];
                }
                None => {
                    out.push('%');
                    rest = after;
                }
            }
        }
        out.push_str(rest);
        out
    }

    fn hid_devices(&self) -> Result<Vec<HidInfo>> {
        Ok(self.hid.clone())
    }

    fn hid_exchange(&mut self, path: &str, out: &[u8], how: &HidTransfer) -> Result<Vec<u8>> {
        self.write_gate("mouse")?;
        self.log.push(format!("hid_exchange {path} {how:?} {}", hex(out)));
        let m = self.mice.get_mut(path).ok_or_else(|| Error::MouseGone(path.into()))?;
        m(out)
    }

    fn rawaccel_driver_version(&self) -> Result<Option<DriverVersion>> {
        Ok(self.rawaccel_version)
    }

    fn read_text(&self, path: &Path) -> Result<Option<String>> {
        Ok(self.files.get(path).cloned())
    }

    fn read_bytes(&self, path: &Path) -> Result<Option<Vec<u8>>> {
        Ok(self.byte_files.get(path).cloned())
    }

    fn write_bytes(&mut self, path: &Path, bytes: &[u8]) -> Result<()> {
        self.write_gate("file")?;
        self.log.push(format!("write_bytes {} {} bytes", path.display(), bytes.len()));
        self.byte_files.insert(path.to_path_buf(), bytes.to_vec());
        Ok(())
    }

    fn rawaccel_read(&self) -> Result<Option<Vec<u8>>> {
        Ok(self.rawaccel_version.map(|_| self.rawaccel_driver.clone()))
    }

    fn rawaccel_write(&mut self, bytes: &[u8]) -> Result<()> {
        self.write_gate("Raw Accel")?;
        if self.rawaccel_version.is_none() {
            return Err(Error::RawAccelMissing("driver not running".into()));
        }
        self.log.push(format!("rawaccel_write {} bytes", bytes.len()));
        self.rawaccel_driver = bytes.to_vec();
        self.rawaccel_byte_writes += 1;
        Ok(())
    }

    fn rawaccel_writer(&mut self, rawaccel_dir: &Path, settings_file: &Path, json: &str) -> Result<()> {
        self.write_gate("Raw Accel")?;
        if self.rawaccel_version.is_none() {
            return Err(Error::RawAccelMissing("driver not running".into()));
        }
        if !self.files.contains_key(&rawaccel_dir.join("writer.exe")) {
            return Err(Error::RawAccelMissing(format!("{} has no writer.exe", rawaccel_dir.display())));
        }
        if let Some(msg) = self.rawaccel_refuse.pop_front() {
            return Err(Error::RawAccelRefused(msg));
        }
        self.log.push(format!("rawaccel_writer {}", settings_file.display()));
        self.rawaccel_writes.push((settings_file.to_path_buf(), json.to_string()));
        Ok(())
    }

    fn is_elevated(&self) -> bool {
        self.elevated
    }
    fn pause_ms(&self, _ms: u64) {}
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" ")
}

/// A fake Pulsar cMouse (X2 CrazyLight family) answering the 17-byte protocol from a settings memory, for tests.
/// Its answers follow the documented frame rules (echoed command, status, address/length echo, checksum).
#[derive(Clone, Debug)]
pub struct CmouseModel {
    pub mem: Vec<u8>,
    pub online: bool,
    pub locked: bool,
    pub link_code: u8,
    pub model_code: u8,
    pub battery: (u8, bool, u16),
    /// every request seen, in order
    pub seen: Vec<Vec<u8>>,
    /// corrupt the checksum of the next answer
    pub corrupt_next: bool,
    /// refuse writes while not locked (status 1)
    pub require_lock: bool,
    /// refuse every write (status 1)
    pub refuse_writes: bool,
    /// the mouse is still linking: the next N online queries answer "offline" (a mouse that just woke up)
    pub waking: u32,
    /// the next N online queries answer "busy" (`[10]` = 1)
    pub busy: u32,
    /// settings reads answer status 1 until the app announced itself (0x02 01) - a dongle that needs it
    pub needs_driver: bool,
    /// the app announced itself (0x02 01, released by 0x02 00)
    pub driver_on: bool,
}

impl CmouseModel {
    /// Pulsar's defaults as the sources describe them: 1000 Hz, 4 stages (400/800/1600/3200), stage 1 active (800),
    /// lift-off 1 mm, wireless 8K link.
    pub fn new() -> Self {
        let mut mem = vec![0u8; 0x100];
        let pair = |v: u8| [v, 0x55u8.wrapping_sub(v)];
        mem[0..2].copy_from_slice(&pair(0x01));
        mem[2..4].copy_from_slice(&pair(4));
        mem[4..6].copy_from_slice(&pair(1));
        mem[0x0A..0x0C].copy_from_slice(&pair(0x01));
        for (i, dpi) in [400u32, 800, 1600, 3200].iter().enumerate() {
            let r = crate::pulsar::encode_dpi_stage(*dpi).unwrap_or([0; 4]);
            mem[0x0C + 4 * i..0x10 + 4 * i].copy_from_slice(&r);
        }
        Self { mem, online: true, locked: false, link_code: 5, model_code: 10, battery: (78, false, 3950), seen: Vec::new(), corrupt_next: false, require_lock: true, refuse_writes: false, waking: 0, busy: 0, needs_driver: false, driver_on: false }
    }

    /// One request → its answer (17 bytes, or `in_len` when the interface is longer).
    pub fn answer(&mut self, req: &[u8]) -> Vec<u8> {
        use crate::pulsar as p;
        self.seen.push(req.to_vec());
        let mut a = [0u8; p::FRAME_LEN];
        a[0] = p::REPORT_ID;
        a[1] = req[1];
        a[3] = req[3];
        a[4] = req[4];
        a[5] = req[5];
        let addr = u16::from_be_bytes([req[3], req[4]]) as usize;
        let len = (req[5] as usize).min(10);
        match req[1] {
            p::CMD_IDENTIFY => {
                a[6..10].copy_from_slice(&req[6..10]);
                a[10] = p::FAMILY_CMOUSE;
                a[11] = self.model_code;
                a[12] = self.link_code;
                a[13] = 1;
            }
            // the real 8K dongle's two forms (measured: the hold answers status 1 and echoes its byte)
            p::CMD_ONLINE if req[5] == 0 => {
                let waking = self.waking > 0;
                self.waking = self.waking.saturating_sub(1);
                a[6] = (self.online && !waking) as u8;
                a[10] = (self.busy > 0) as u8;
                self.busy = self.busy.saturating_sub(1);
            }
            p::CMD_ONLINE => {
                a[2] = 1;
                if self.online {
                    self.locked = req[6] == 1;
                    a[6] = req[6];
                }
            }
            p::CMD_DRIVER => {
                self.driver_on = req[6] == 1;
                a[6] = req[6];
            }
            p::CMD_BATTERY => {
                a[6] = self.battery.0;
                a[7] = self.battery.1 as u8;
                a[8..10].copy_from_slice(&self.battery.2.to_be_bytes());
            }
            p::CMD_READ => {
                if self.online && (self.driver_on || !self.needs_driver) && addr + len <= self.mem.len() {
                    a[6..6 + len].copy_from_slice(&self.mem[addr..addr + len]);
                } else {
                    a[2] = 1;
                }
            }
            p::CMD_WRITE => {
                if self.refuse_writes || (self.require_lock && !self.locked) || !self.online || addr + len > self.mem.len() {
                    a[2] = 1;
                } else {
                    self.mem[addr..addr + len].copy_from_slice(&req[6..6 + len]);
                    // the ack is the request itself (PulsarBattery's MatchesWriteAck)
                    a[6..6 + len].copy_from_slice(&req[6..6 + len]);
                }
            }
            _ => a[2] = 1,
        }
        a[16] = p::checksum(&a[..16]);
        if self.corrupt_next {
            self.corrupt_next = false;
            a[16] ^= 0xFF;
        }
        a.to_vec()
    }
}

impl Default for CmouseModel {
    fn default() -> Self {
        Self::new()
    }
}
