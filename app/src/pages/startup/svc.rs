//! The Startup page's link to `bu-startup`: the real layer (Windows; its list is read off the UI thread) or the FAKE one
//! seeded with the drawing's sample list (menu-v22 `SUP`, 15 entries: 8 normal, 3 tasks, 4 services; OBS off; impacts
//! high / medium / low as drawn). User folders are `C:\Users\someone\…` (no real names).

use bu_startup::fake::FakeOs;
use bu_startup::os::{RawService, RawTask, ServiceStart, TaskTrigger};
use bu_startup::saved::{State as SavedState, Target};
use bu_startup::{Change, Hive, RegView, Startup, StartupEntry, StartupError, StartupList, RUN};

pub enum Svc {
    /// shared, so the list can be read again on a helper thread like the real one (Order 047)
    Fake(std::sync::Arc<Startup<FakeOs>>),
    #[cfg(windows)]
    Real(Startup<crate::admin::proxy::StartupOs>),
}

impl Svc {
    /// Windows, with the admin rows' changes (HKLM Run / Startup folder flags, tasks, services) going to the app's elevated
    /// copy: one admin prompt per switch (Order 039).
    #[cfg(windows)]
    pub fn real() -> Svc {
        let os = std::sync::Arc::new(bu_startup::real::RealOs::new());
        Svc::Real(Startup::new(crate::admin::proxy::StartupOs::new(os, crate::admin::client::admin())))
    }

    pub fn list(&self) -> StartupList {
        match self {
            Svc::Fake(s) => s.list(),
            #[cfg(windows)]
            Svc::Real(s) => s.list(),
        }
    }
    /// Order 047: the list read as a job for a helper thread (Task Scheduler, the services, every program's version info:
    /// 0.2 - 1.5 s on a real PC) - the real one reads with its own copy of the OS layer, as the tab's open does.
    pub fn lister(&self) -> Box<dyn FnOnce() -> StartupList + Send> {
        match self {
            Svc::Fake(s) => {
                let s = s.clone();
                Box::new(move || s.list())
            }
            #[cfg(windows)]
            Svc::Real(_) => Box::new(|| Startup::new(bu_startup::real::RealOs::new()).list()),
        }
    }
    /// Order 047: the same service for another thread - the fake shared (its state is the "PC"); None for the real one
    /// (that thread makes its own: every state lives in Windows).
    pub fn share(&self) -> Option<Svc> {
        match self {
            Svc::Fake(s) => Some(Svc::Fake(s.clone())),
            #[cfg(windows)]
            Svc::Real(_) => None,
        }
    }
    pub fn set(&self, e: &StartupEntry, on: bool) -> Result<Change, StartupError> {
        match self {
            Svc::Fake(s) => s.set_enabled(e, on),
            #[cfg(windows)]
            Svc::Real(s) => s.set_enabled(e, on),
        }
    }
    /// One row's state now (the reset line's `current`; one read).
    pub fn state_of(&self, t: &Target) -> Result<SavedState, StartupError> {
        match self {
            Svc::Fake(s) => s.state_of(t),
            #[cfg(windows)]
            Svc::Real(s) => s.state_of(t),
        }
    }
    /// Put one row back to a state (the reset line's `apply`).
    pub fn put_back(&self, t: &Target, st: SavedState) -> Result<(), StartupError> {
        match self {
            Svc::Fake(s) => s.put_back(t, st),
            #[cfg(windows)]
            Svc::Real(s) => s.put_back(t, st),
        }
    }
    pub fn fake(&self) -> Option<&FakeOs> {
        match self {
            Svc::Fake(s) => Some(s.os()),
            #[cfg(windows)]
            Svc::Real(_) => None,
        }
    }

    /// The fake at the drawing's sample list (not admin, like a normal start).
    pub fn sample() -> Svc {
        const U: &str = r"C:\Users\someone";
        let p = |s: &str| s.replace("%U%", U);
        // (name, publisher, exe, args, hive) - the drawing's normal rows; where they live as on a real PC (Windows Security and
        // the NVIDIA App in the machine-wide Run key, the rest per user) - the crate lists per-user ones first
        let normal: [(&str, &str, &str, &str, Hive); 8] = [
            ("Discord", "Discord Inc.", r"%U%\AppData\Local\Discord\Update.exe", " --processStart Discord.exe", Hive::CurrentUser),
            ("Steam", "Valve Corporation", r"C:\Program Files (x86)\Steam\steam.exe", " -silent", Hive::CurrentUser),
            ("OneDrive", "Microsoft Corporation", r"C:\Program Files\Microsoft OneDrive\OneDrive.exe", " /background", Hive::CurrentUser),
            ("Windows Security notification icon", "Microsoft Windows", r"C:\Windows\System32\SecurityHealthSystray.exe", "", Hive::LocalMachine),
            ("Spotify", "Spotify AB", r"%U%\AppData\Roaming\Spotify\Spotify.exe", " /minimized", Hive::CurrentUser),
            ("NVIDIA App", "NVIDIA Corporation", r"C:\Program Files\NVIDIA Corporation\NVIDIA App\CEF\NVIDIA App.exe", " -s", Hive::LocalMachine),
            ("OBS Studio", "OBS Project", r"C:\Program Files\obs-studio\bin\64bit\obs64.exe", " --minimize-to-tray", Hive::CurrentUser),
            ("Wootility", "Wooting", r"%U%\AppData\Local\Programs\wootility\Wootility.exe", " --hidden", Hive::CurrentUser),
        ];
        let mut f = FakeOs::new().env("windir", r"C:\Windows").env("SystemRoot", r"C:\Windows");
        for (name, publ, exe, args, hive) in normal.iter() {
            let exe = p(exe);
            // Discord starts through its Squirrel launcher: the row keeps the Run value's own name (the crate's rule)
            let desc = if *name == "Discord" { "Update" } else { name };
            f = f.file(&exe, publ, desc).run(*hive, RegView::Bits64, RUN, name, &format!("\"{exe}\"{args}"));
        }
        // OBS Studio is off (Task Manager's 03 flag)
        f = f.binary(Hive::CurrentUser, r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run", "OBS Studio", &[3, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8]);
        let tasks = [
            ("GoogleUpdateTaskMachineUA", "Google LLC", r"C:\Program Files (x86)\Google\Update\GoogleUpdate.exe"),
            ("Adobe Acrobat Update Task", "Adobe Inc.", r"C:\Program Files (x86)\Common Files\Adobe\ARM\1.0\AdobeARM.exe"),
            ("MicrosoftEdgeUpdateTaskMachineCore", "Microsoft Corporation", r"C:\Program Files (x86)\Microsoft\EdgeUpdate\MicrosoftEdgeUpdate.exe"),
        ];
        for (name, publ, exe) in tasks {
            f = f.file(exe, publ, name).task(RawTask {
                path: format!("\\{name}"),
                name: name.into(),
                enabled: true,
                triggers: vec![TaskTrigger::Logon],
                command: Some(exe.into()),
                arguments: None,
                author: Some(publ.into()),
            });
        }
        let services = [
            ("NVDisplay.ContainerLocalSystem", "NVIDIA LocalSystem Container", "NVIDIA Corporation", r"C:\Windows\System32\DriverStore\FileRepository\nv_dispi.inf\Display.NvContainer\NVDisplay.Container.exe", ""),
            ("EpicOnlineServices", "EpicOnlineServices", "Epic Games, Inc.", r"C:\Program Files (x86)\Epic Games\Epic Online Services\EpicOnlineServices.exe", ""),
            ("WinDefend", "Microsoft Defender Antivirus Service", "Microsoft Windows", r"C:\ProgramData\Microsoft\Windows Defender\Platform\4.18.25080.5-0\MsMpEng.exe", ""),
            ("Audiosrv", "Windows Audio", "Microsoft Windows", r"C:\Windows\System32\svchost.exe", " -k LocalServiceNetworkRestricted -p"),
        ];
        for (name, disp, publ, exe, args) in services {
            f = f.file(exe, publ, disp).service(RawService {
                name: name.into(),
                display_name: disp.into(),
                start: ServiceStart::Automatic,
                delayed: false,
                image_path: Some(format!("\"{exe}\"{args}")),
            });
        }
        // the boot report: Task Manager's thresholds (High > 1 s CPU, Medium 300 - 1000 ms, Low < 300 ms)
        let imp = |exe: &str, cpu_ms: u64| format!("<Process Name=\"{exe}\" PID=\"1\" StartedInTraceSec=\"1\"><DiskUsage Units=\"bytes\">1000</DiskUsage><CpuUsage Units=\"us\">{}</CpuUsage></Process>", cpu_ms * 1000);
        let mut xml = String::from("<StartupData><Startup>");
        for (i, (_, _, exe, _, _)) in normal.iter().enumerate() {
            let ms = [1500, 1500, 1500, 50, 600, 600, 600, 50][i];
            xml.push_str(&imp(&p(exe), ms));
        }
        for (exe, ms) in [(tasks[0].2, 50), (tasks[1].2, 50), (tasks[2].2, 50), (services[0].3, 600), (services[1].3, 50), (services[2].3, 600), (services[3].3, 50)] {
            xml.push_str(&imp(exe, ms));
        }
        xml.push_str("</Startup></StartupData>");
        f = f.impact(Ok(vec![xml]));
        Svc::Fake(std::sync::Arc::new(Startup::new(f)))
    }
}
