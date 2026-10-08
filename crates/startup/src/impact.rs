//! Startup impact, measured the way Task Manager does it.
//!
//! Source of the numbers: Windows' Diagnostic Infrastructure writes a boot trace (BootCKCL.etl) and from it an XML report per
//! user in `%windir%\System32\wdi\LogFiles\StartupInfo\<SID>_StartupInfo<n>.xml` (winaero.com "how the Windows 8 Task Manager
//! calculates startup impact"; winhelponline.com "task-manager-startup-impact-calculated-bootckcl"). Format (verified against the
//! parser + sample in fox-it/dissect.target `plugins/os/windows/startupinfo.py`):
//! `<Process Name="C:\…\x.exe" PID=".." StartedInTraceSec=".."> … <DiskUsage Units="bytes">325120</DiskUsage>
//! <CpuUsage Units="us">32024</CpuUsage> … </Process>`.
//! Thresholds (same sources): High = more than 1 s CPU or more than 3 MB disk I/O; Medium = 300–1000 ms CPU or 300 KB–3 MB disk;
//! Low = under 300 ms CPU and under 300 KB disk; not in the report = "Not measured".
//! **unclear:** whether "MB/KB" means 1000- or 1024-based; we use 1024 (KiB/MiB).

use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Impact {
    Low,
    Medium,
    High,
}

/// One process's boot cost from the report.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cost {
    pub cpu_us: u64,
    pub disk_bytes: u64,
}

pub fn classify(cost: Cost) -> Impact {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;
    if cost.cpu_us > 1_000_000 || cost.disk_bytes > 3 * MB {
        Impact::High
    } else if cost.cpu_us >= 300_000 || cost.disk_bytes >= 300 * KB {
        Impact::Medium
    } else {
        Impact::Low
    }
}

/// Exe path (lower case) → summed cost of every process with that path in one report.
pub fn parse_report(xml: &str) -> HashMap<String, Cost> {
    let mut out: HashMap<String, Cost> = HashMap::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<Process ") {
        rest = &rest[start..];
        let end = rest.find("</Process>").unwrap_or(rest.len());
        let block = &rest[..end];
        if let Some(name) = attr(block, "Name") {
            let cost = Cost {
                cpu_us: element_number(block, "CpuUsage").unwrap_or(0),
                disk_bytes: element_number(block, "DiskUsage").unwrap_or(0),
            };
            let e = out.entry(unescape(&name).to_lowercase()).or_default();
            e.cpu_us += cost.cpu_us;
            e.disk_bytes += cost.disk_bytes;
        }
        rest = &rest[end.min(rest.len())..];
        if rest.starts_with("</Process>") {
            rest = &rest["</Process>".len()..];
        }
    }
    out
}

fn attr(block: &str, name: &str) -> Option<String> {
    let head_end = block.find('>')?;
    let head = &block[..head_end];
    let key = format!(" {name}=\"");
    let s = head.find(&key)? + key.len();
    let e = head[s..].find('"')? + s;
    Some(head[s..e].to_string())
}

fn element_number(block: &str, tag: &str) -> Option<u64> {
    let open = block.find(&format!("<{tag}"))?;
    let after = &block[open..];
    let gt = after.find('>')? + 1;
    let close = after.find(&format!("</{tag}>"))?;
    after.get(gt..close)?.trim().parse().ok()
}

fn unescape(s: &str) -> String {
    s.replace("&quot;", "\"").replace("&apos;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thresholds() {
        assert_eq!(classify(Cost { cpu_us: 0, disk_bytes: 0 }), Impact::Low);
        assert_eq!(classify(Cost { cpu_us: 299_999, disk_bytes: 300 * 1024 - 1 }), Impact::Low);
        assert_eq!(classify(Cost { cpu_us: 300_000, disk_bytes: 0 }), Impact::Medium);
        assert_eq!(classify(Cost { cpu_us: 0, disk_bytes: 300 * 1024 }), Impact::Medium);
        assert_eq!(classify(Cost { cpu_us: 1_000_000, disk_bytes: 3 * 1024 * 1024 }), Impact::Medium);
        assert_eq!(classify(Cost { cpu_us: 1_000_001, disk_bytes: 0 }), Impact::High);
        assert_eq!(classify(Cost { cpu_us: 0, disk_bytes: 3 * 1024 * 1024 + 1 }), Impact::High);
    }

    #[test]
    fn parses_the_documented_sample() {
        let xml = r#"<?xml version="1.0"?><StartupData><Startup>
<Process Name="C:\Windows\System32\SecurityHealthSystray.exe" PID="6208" StartedInTraceSec="48.500">
    <StartTime>2020/09/11:18:12:48.6685573</StartTime>
    <CommandLine><![CDATA["C:\Windows\System32\SecurityHealthSystray.exe" ]]></CommandLine>
    <DiskUsage Units="bytes">325120</DiskUsage>
    <CpuUsage Units="us">32024</CpuUsage>
    <ParentPID>5592</ParentPID>
</Process>
<Process Name="C:\Program Files\A &amp; B\a.exe" PID="1" StartedInTraceSec="1.0">
    <DiskUsage Units="bytes">10</DiskUsage><CpuUsage Units="us">5</CpuUsage>
</Process>
<Process Name="C:\Program Files\A &amp; B\a.exe" PID="2" StartedInTraceSec="2.0">
    <DiskUsage Units="bytes">10</DiskUsage><CpuUsage Units="us">5</CpuUsage>
</Process>
</Startup></StartupData>"#;
        let m = parse_report(xml);
        assert_eq!(m[r"c:\windows\system32\securityhealthsystray.exe"], Cost { cpu_us: 32024, disk_bytes: 325120 });
        assert_eq!(m[r"c:\program files\a & b\a.exe"], Cost { cpu_us: 10, disk_bytes: 20 });
        assert_eq!(classify(m[r"c:\windows\system32\securityhealthsystray.exe"]), Impact::Medium);
    }

    #[test]
    fn garbage_is_empty_not_a_panic() {
        assert!(parse_report("").is_empty());
        assert!(parse_report("<Process Name=\"x").is_empty() || parse_report("<Process Name=\"x").len() <= 1);
        assert!(parse_report("<Process >").is_empty());
    }
}
