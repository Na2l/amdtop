//! Disk I/O from `/proc/diskstats`, in the shape btop shows it.

use std::collections::HashMap;
use std::path::Path;

use crate::util;

/// Per-device I/O rates and utilisation for one sample.
#[derive(Debug, Clone, Default)]
pub struct DiskStats {
    pub name: String,
    pub model: String,
    /// Bytes per second.
    pub read_bps: f64,
    pub write_bps: f64,
    pub read_iops: f64,
    pub write_iops: f64,
    /// Fraction of wall time the device had I/O in flight (0..1).
    pub busy: f64,
    pub size: u64,
}

#[derive(Debug, Clone, Copy, Default)]
struct Raw {
    sectors_read: u64,
    sectors_written: u64,
    reads: u64,
    writes: u64,
    ms_io: u64,
}

impl Raw {
    fn from_fields(fields: &[&str]) -> Raw {
        let get = |index: usize| -> u64 {
            fields
                .get(index)
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0)
        };
        Raw {
            reads: get(0),
            sectors_read: get(2),
            writes: get(4),
            sectors_written: get(6),
            ms_io: get(9),
        }
    }
}

pub struct DiskReader {
    prev: HashMap<String, Raw>,
    last_wall: f64,
}

impl DiskReader {
    pub fn new() -> Self {
        DiskReader {
            prev: HashMap::new(),
            last_wall: 0.0,
        }
    }

    pub fn sample(&mut self, wall: f64) -> Vec<DiskStats> {
        let dt = if self.last_wall > 0.0 {
            (wall - self.last_wall).max(1e-6)
        } else {
            0.0
        };
        self.last_wall = wall;
        let Some(text) = util::read_string("/proc/diskstats") else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut seen = Vec::new();
        for line in text.lines() {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 14 {
                continue;
            }
            let name = fields[2].to_string();
            if !interesting(&name) {
                continue;
            }
            let raw = Raw::from_fields(&fields[3..]);
            seen.push(name.clone());
            let previous = self.prev.get(&name).copied().unwrap_or(raw);
            let mut stats = DiskStats {
                name: name.clone(),
                model: device_model(&name),
                size: device_size(&name),
                ..Default::default()
            };
            if dt > 0.0 {
                let d_read = raw.sectors_read.saturating_sub(previous.sectors_read);
                let d_write = raw.sectors_written.saturating_sub(previous.sectors_written);
                stats.read_bps = d_read as f64 * 512.0 / dt;
                stats.write_bps = d_write as f64 * 512.0 / dt;
                stats.read_iops = raw.reads.saturating_sub(previous.reads) as f64 / dt;
                stats.write_iops = raw.writes.saturating_sub(previous.writes) as f64 / dt;
                let busy_ms = raw.ms_io.saturating_sub(previous.ms_io) as f64;
                stats.busy = crate::util::clamp(busy_ms / (dt * 1000.0), 0.0, 1.0);
            }
            self.prev.insert(name, raw);
            out.push(stats);
        }
        self.prev.retain(|name, _| seen.contains(name));
        out.sort_by(|a, b| {
            (b.read_bps + b.write_bps)
                .partial_cmp(&(a.read_bps + a.write_bps))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        out
    }
}

/// Skip virtual and pseudo devices, the way btop does.
fn interesting(name: &str) -> bool {
    const SKIP: [&str; 6] = ["loop", "ram", "zram", "dm-", "sr", "md"];
    if SKIP.iter().any(|prefix| name.starts_with(prefix)) {
        return false;
    }
    // Only whole devices (they have a directory in /sys/block).
    Path::new("/sys/block").join(name).exists()
}

fn device_model(name: &str) -> String {
    util::read_trim(format!("/sys/block/{}/device/model", name))
        .or_else(|| util::read_trim(format!("/sys/block/{}/device/name", name)))
        .map(|model| model.trim().to_string())
        .unwrap_or_default()
}

fn device_size(name: &str) -> u64 {
    util::read_u64(format!("/sys/block/{}/size", name))
        .map(|sectors| sectors * 512)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_diskstats_fields() {
        let line = "   8       0 sda 100 0 2000 50 60 0 4000 70 0 120 90 0 0 0 0";
        let fields: Vec<&str> = line.split_whitespace().collect();
        let raw = Raw::from_fields(&fields[3..]);
        assert_eq!(raw.reads, 100);
        assert_eq!(raw.sectors_read, 2000);
        assert_eq!(raw.writes, 60);
        assert_eq!(raw.sectors_written, 4000);
        assert_eq!(raw.ms_io, 120);
    }

    #[test]
    fn skips_virtual_devices() {
        assert!(!interesting("loop0"));
        assert!(!interesting("zram0"));
        assert!(!interesting("dm-0"));
        assert!(!interesting("ram0"));
    }
}
