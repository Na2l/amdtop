//! Small odds and ends btop puts in its top bar: battery, hostname and clock.

use std::path::PathBuf;

use crate::util;

#[derive(Debug, Clone, Default)]
pub struct BatteryInfo {
    pub name: String,
    pub capacity: Option<f64>,
    pub status: String,
    pub power_w: Option<f64>,
    pub energy_wh: Option<f64>,
    pub energy_full_wh: Option<f64>,
}

/// First battery exposed by the kernel, if this is a laptop.
pub fn read_battery() -> Option<BatteryInfo> {
    let entries = std::fs::read_dir("/sys/class/power_supply").ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("BAT") && !name.starts_with("bat") {
            continue;
        }
        let dir: PathBuf = entry.path();
        let capacity = util::read_u64(dir.join("capacity")).map(|value| value as f64);
        let micro = |file: &str| util::read_u64(dir.join(file)).map(|value| value as f64 / 1e6);
        let power_w = micro("power_now");
        let energy_wh = micro("energy_now");
        let energy_full_wh = micro("energy_full");
        let status = util::read_trim(dir.join("status")).unwrap_or_default();
        return Some(BatteryInfo {
            name,
            capacity,
            status,
            power_w: power_w.filter(|value| *value > 0.0),
            energy_wh,
            energy_full_wh,
        });
    }
    None
}

pub fn hostname() -> String {
    util::read_trim("/proc/sys/kernel/hostname").unwrap_or_default()
}

/// Local wall clock as `HH:MM:SS` (uses libc so timezones are handled).
pub fn clock_string() -> String {
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&now, &mut tm).is_null() {
            return String::new();
        }
        format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
    }
}

/// Uptime formatted the way btop shows it: `2d 23:38`.
pub fn uptime_string(seconds: f64) -> String {
    let total = seconds.max(0.0) as u64;
    let days = total / 86400;
    let hours = (total % 86400) / 3600;
    let minutes = (total % 3600) / 60;
    if days > 0 {
        format!("{}d {:02}:{:02}", days, hours, minutes)
    } else {
        format!("{:02}:{:02}", hours, minutes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_uptime_like_btop() {
        assert_eq!(uptime_string(90000.0), "1d 01:00");
        assert_eq!(uptime_string(3725.0), "01:02");
    }

    #[test]
    fn clock_looks_like_a_clock() {
        let clock = clock_string();
        assert_eq!(clock.len(), 8, "got {:?}", clock);
        assert_eq!(clock.matches(':').count(), 2);
    }
}
