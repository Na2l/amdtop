//! amdgpu discovery and sysfs sampling (busy %, VRAM/GTT, clocks, power).

use std::path::{Path, PathBuf};

use crate::collect::cpu::hwmon_sensor_temps;
use crate::collect::GpuSample;
use crate::util;

/// A discovered amdgpu DRM card.
#[derive(Debug, Clone)]
pub struct GpuDevice {
    pub card: String,
    pub device: PathBuf,
    pub pci_slot: String,
    pub pci_id: String,
    pub driver: String,
    pub primary_minor: Option<u32>,
}

/// Marketing names for the AMD iGPU PCI device ids seen on Strix family parts.
const PCI_NAMES: &[(&str, &str)] = &[
    ("1002:1586", "Radeon 8060S (gfx1151, Strix Halo)"),
    ("1002:1585", "Radeon 8050S (gfx1150, Strix Point)"),
    ("1002:1586 ", ""),
    ("1002:1500", "Radeon 780M (gfx1103)"),
    ("1002:1503", "Radeon 760M (gfx1103)"),
    ("1002:1507", "Radeon 680M (gfx1103)"),
    ("1002:1681", "Radeon 890M (gfx1151)"),
    ("1002:13c0", "Radeon 660M (gfx1102)"),
    ("1002:164e", "Radeon 610M (gfx1102)"),
    ("1002:744c", "Radeon RX 7900 XTX (gfx1100)"),
    ("1002:7480", "Radeon RX 7900 (gfx1100)"),
    ("1002:7550", "Radeon RX 9070 (gfx1201)"),
];

/// Find the first amdgpu card under `/sys/class/drm`.
pub fn discover() -> Option<GpuDevice> {
    discover_all().into_iter().next()
}

/// All amdgpu cards, ordered by card index.
pub fn discover_all() -> Vec<GpuDevice> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
        return out;
    };
    let mut cards: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        // Skip connector directories such as `card0-DP-1`.
        if name.starts_with("card") && !name.contains('-') && !name.contains("render") {
            cards.push(name);
        }
    }
    cards.sort();
    for card in cards {
        let device = PathBuf::from("/sys/class/drm").join(&card).join("device");
        let vendor = util::read_trim(device.join("vendor")).unwrap_or_default();
        if vendor != "0x1002" {
            continue;
        }
        let pci_id = util::read_trim(device.join("device"))
            .map(|device_id| format!("{}:{}", vendor.trim_start_matches("0x"), device_id.trim_start_matches("0x")))
            .unwrap_or_default();
        let pci_slot = util::read_trim(device.join("uevent"))
            .map(|_| String::new())
            .unwrap_or_default();
        let pci_slot = util::read_string(device.join("uevent"))
            .and_then(|text| {
                text.lines()
                    .find_map(|line| line.strip_prefix("PCI_SLOT_NAME="))
                    .map(|value| value.to_string())
            })
            .unwrap_or(pci_slot);
        let driver = util::read_string(device.join("uevent"))
            .and_then(|text| {
                text.lines()
                    .find_map(|line| line.strip_prefix("DRIVER="))
                    .map(|value| value.to_string())
            })
            .unwrap_or_else(|| "amdgpu".to_string());
        let primary_minor = util::read_trim(format!("/sys/class/drm/{}/dev", card))
            .and_then(|dev| dev.split(':').nth(1).and_then(|minor| minor.parse::<u32>().ok()));
        out.push(GpuDevice {
            card,
            device,
            pci_slot,
            pci_id,
            driver,
            primary_minor,
        });
    }
    out
}

pub fn marketing_name(pci_id: &str) -> Option<&'static str> {
    let needle = pci_id.to_ascii_lowercase();
    PCI_NAMES
        .iter()
        .find(|(id, _)| *id == needle.as_str())
        .map(|(_, name)| *name)
        .filter(|name| !name.is_empty())
}

/// Read every counter for one card.
pub fn sample(dev: &GpuDevice) -> GpuSample {
    let device = &dev.device;
    let mut sample = GpuSample {
        card: dev.card.clone(),
        pci_slot: dev.pci_slot.clone(),
        pci_id: dev.pci_id.clone(),
        marketing: marketing_name(&dev.pci_id)
            .map(|name| name.to_string())
            .unwrap_or_else(|| format!("AMD GPU [{}]", dev.pci_id)),
        driver: dev.driver.clone(),
        ..Default::default()
    };

    sample.busy = util::read_u64(device.join("gpu_busy_percent")).map(|v| v as f64 / 100.0);
    sample.mem_busy = util::read_u64(device.join("mem_busy_percent")).map(|v| v as f64 / 100.0);

    // Every mem_info_* counter the driver happens to export.
    let mut counters: Vec<(String, u64)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(device) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(counter) = name.strip_prefix("mem_info_") {
                if let Some(value) = util::read_u64(entry.path()) {
                    counters.push((counter.to_string(), value));
                }
            }
        }
    }
    counters.sort();
    for (name, value) in counters {
        match name.as_str() {
            "vram_total" => sample.vram_total = Some(value),
            "vram_used" => sample.vram_used = Some(value),
            "vis_vram_total" => sample.vis_vram_total = Some(value),
            "vis_vram_used" => sample.vis_vram_used = Some(value),
            "gtt_total" => sample.gtt_total = Some(value),
            "gtt_used" => sample.gtt_used = Some(value),
            other => sample.extra_mem.push((other.to_string(), None, None)),
        }
    }

    // Clocks from pp_dpm_sclk / pp_dpm_mclk ("* 13: 2900Mhz" marks the current one).
    let (sclk, sclk_min, sclk_max) = read_dpm_clocks(&device.join("pp_dpm_sclk"));
    sample.sclk = sclk;
    sample.sclk_min = sclk_min;
    sample.sclk_max = sclk_max;
    let (mclk, mclk_min, mclk_max) = read_dpm_clocks(&device.join("pp_dpm_mclk"));
    sample.mclk = mclk;
    sample.mclk_min = mclk_min;
    sample.mclk_max = mclk_max;

    // hwmon: temperatures, power, fan.
    if let Some(hwmon) = find_hwmon(device) {
        sample.temps = hwmon_sensor_temps(&hwmon);
        sample.power_w = util::read_u64(hwmon.join("power1_average"))
            .or_else(|| util::read_u64(hwmon.join("power1_input")))
            .map(|micro| micro as f64 / 1_000_000.0);
        sample.power_cap = util::read_u64(hwmon.join("power1_cap")).map(|micro| micro as f64 / 1_000_000.0);
        sample.fan_rpm = util::read_u64(hwmon.join("fan1_input")).map(|v| v as f64);
        sample.fan_percent = util::read_u64(hwmon.join("pwm1")).map(|pwm| pwm as f64 / 255.0);
    }
    sample
}

fn find_hwmon(device: &Path) -> Option<PathBuf> {
    let dir = device.join("hwmon");
    let entries = std::fs::read_dir(&dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.join("name").exists() || path.join("temp1_input").exists() {
            return Some(path);
        }
    }
    None
}

/// Parse `pp_dpm_*` output: returns `(current, min, max)` in MHz.
pub fn read_dpm_clocks(path: &Path) -> (Option<f64>, Option<f64>, Option<f64>) {
    let Some(text) = util::read_string(path) else {
        return (None, None, None);
    };
    from_dpm_text(&text)
}

pub fn from_dpm_text(text: &str) -> (Option<f64>, Option<f64>, Option<f64>) {
    let mut current = None;
    let mut values: Vec<f64> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some((_, rest)) = line.split_once(':') else { continue };
        let rest = rest.trim();
        let is_current = rest.contains('*');
        let digits: String = rest
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        let Ok(value) = digits.parse::<f64>() else { continue };
        values.push(value);
        if is_current {
            current = Some(value);
        }
    }
    let min = values.iter().copied().fold(None, |a: Option<f64>, v| Some(a.map_or(v, |x: f64| x.min(v))));
    let max = values.iter().copied().fold(None, |a: Option<f64>, v| Some(a.map_or(v, |x: f64| x.max(v))));
    (current.or(min), min, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dpm_clocks() {
        let text = "0: 213Mhz\n1: 420Mhz\n2: 1200Mhz *\n3: 2900Mhz\n";
        let (current, min, max) = from_dpm_text(text);
        assert_eq!(current, Some(1200.0));
        assert_eq!(min, Some(213.0));
        assert_eq!(max, Some(2900.0));
    }

    #[test]
    fn parses_dpm_with_sleep_and_range() {
        let text = "0: 420Mhz\n3: 1387Mhz *\n";
        let (current, min, max) = from_dpm_text(text);
        assert_eq!(current, Some(1387.0));
        assert_eq!(min, Some(420.0));
    }

    #[test]
    fn maps_strix_halo_name() {
        assert!(marketing_name("1002:1586").unwrap().contains("Strix Halo"));
        assert_eq!(marketing_name("1002:dead"), None);
    }
}
