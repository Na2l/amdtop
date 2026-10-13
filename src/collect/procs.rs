//! Process table: `top`-style fields plus per-client amdgpu memory.
//!
//! Per-process GPU memory comes from `/proc/<pid>/fdinfo/<fd>` as standardised
//! by `Documentation/gpu/drm-usage-stats.rst`. amdgpu prints, per DRM client:
//!
//! ```text
//! drm-driver: amdgpu
//! drm-client-id: 12
//! drm-pdev: 0000:c5:00.0
//! drm-total-vram: 0 KiB
//! drm-resident-vram: 1234 KiB
//! drm-total-gtt: 987654 KiB
//! drm-resident-gtt: 987654 KiB
//! amd-requested-gtt: 987654 KiB
//! amd-evicted-vram: 0 KiB
//! drm-engine-gfx: 123456789 ns
//! ```
//!
//! Regions are `vram`, `gtt`, `cpu` (plus `gds`/`gws`/`oa` which we ignore).
//! Several file descriptors can belong to one client, so usage is deduplicated
//! by `(drm-pdev, drm-client-id)`.

#![allow(dead_code)] // the collectors mirror every kernel counter, not all are shown yet

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::time::Instant;

use crate::collect::ProcInfo;
use crate::util;

/// Aggregated GPU usage for one process.
#[derive(Debug, Clone, Default)]
pub struct GpuProcStats {
    pub vram: u64,
    pub vram_requested: u64,
    pub gtt: u64,
    pub gtt_requested: u64,
    pub sys: u64,
    pub evicted_vram: u64,
    pub clients: usize,
    pub engines: HashMap<String, u64>,
    pub driver: String,
}

/// One parsed fdinfo blob.
#[derive(Debug, Clone, Default)]
pub struct FdInfo {
    pub driver: String,
    pub client_id: Option<u64>,
    pub pdev: String,
    pub total: HashMap<String, u64>,
    pub resident: HashMap<String, u64>,
    pub shared: HashMap<String, u64>,
    pub requested: HashMap<String, u64>,
    pub evicted: HashMap<String, u64>,
    pub engines: HashMap<String, u64>,
}

/// Parse a DRM fdinfo blob into structured stats.
pub fn parse_fdinfo(text: &str) -> FdInfo {
    let mut info = FdInfo::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        match key {
            "drm-driver" => info.driver = value.to_string(),
            "drm-client-id" => info.client_id = value.parse().ok(),
            "drm-pdev" => info.pdev = value.to_string(),
            _ => {}
        }
        if let Some(region) = key.strip_prefix("drm-total-") {
            if let Some(bytes) = util::parse_size_with_unit(value) {
                info.total.insert(region.to_string(), bytes);
            }
        } else if let Some(region) = key.strip_prefix("drm-resident-") {
            if let Some(bytes) = util::parse_size_with_unit(value) {
                info.resident.insert(region.to_string(), bytes);
            }
        } else if let Some(region) = key.strip_prefix("drm-shared-") {
            if let Some(bytes) = util::parse_size_with_unit(value) {
                info.shared.insert(region.to_string(), bytes);
            }
        } else if let Some(region) = key.strip_prefix("amd-requested-") {
            if let Some(bytes) = util::parse_size_with_unit(value) {
                info.requested.insert(region.to_string(), bytes);
            }
        } else if let Some(region) = key.strip_prefix("amd-evicted-") {
            if let Some(bytes) = util::parse_size_with_unit(value) {
                info.evicted.insert(region.to_string(), bytes);
            }
        } else if let Some(engine) = key.strip_prefix("drm-engine-") {
            // Value looks like "12345 ns".
            let number: String = value
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if let Ok(ns) = number.parse::<u64>() {
                *info.engines.entry(engine.to_string()).or_insert(0) += ns;
            }
        } else if let Some(region) = key.strip_prefix("drm-memory-") {
            // Legacy amdgpu alias of drm-resident-<region>.
            if let Some(bytes) = util::parse_size_with_unit(value) {
                info.resident.entry(region.to_string()).or_insert(bytes);
            }
        }
    }
    info
}

/// Merge one client's fdinfo into the process totals.
pub fn merge_fdinfo(total: &mut GpuProcStats, info: &FdInfo) {
    let pick = |map: &HashMap<String, u64>, region: &str| map.get(region).copied().unwrap_or(0);
    total.vram += pick(&info.resident, "vram");
    total.gtt += pick(&info.resident, "gtt");
    total.sys += pick(&info.resident, "cpu");
    total.vram_requested += pick(&info.total, "vram");
    total.gtt_requested += pick(&info.total, "gtt");
    total.evicted_vram += pick(&info.evicted, "vram");
    if total.vram_requested == 0 {
        total.vram_requested = pick(&info.requested, "vram");
    }
    if total.gtt_requested == 0 {
        total.gtt_requested = pick(&info.requested, "gtt");
    }
    if !info.driver.is_empty() {
        total.driver = info.driver.clone();
    }
    for (engine, ns) in &info.engines {
        *total.engines.entry(engine.clone()).or_insert(0) += *ns;
    }
}

/// Raw fields read from `/proc/<pid>/stat`.
#[derive(Debug, Clone, Default)]
pub struct ProcStat {
    pub pid: u32,
    pub comm: String,
    pub state: char,
    pub ppid: u32,
    pub utime: u64,
    pub stime: u64,
    pub priority: i32,
    pub nice: i32,
    pub threads: u32,
    pub starttime: u64,
    pub vsize: u64,
    pub rss_pages: u64,
}

/// Parse `/proc/<pid>/stat`, handling `comm` values that contain spaces.
pub fn parse_stat(text: &str) -> Option<ProcStat> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    if close < open {
        return None;
    }
    let comm = &text[open + 1..close];
    let rest: Vec<&str> = text[close + 1..].split_whitespace().collect();
    let get = |index: usize| rest.get(index).copied().unwrap_or("0");
    Some(ProcStat {
        pid: text[..open].trim().parse().ok()?,
        comm: comm.to_string(),
        state: get(0).chars().next().unwrap_or('?'),
        ppid: get(1).parse().unwrap_or(0),
        utime: get(11).parse().unwrap_or(0),
        stime: get(12).parse().unwrap_or(0),
        priority: get(15).parse().unwrap_or(0),
        nice: get(16).parse().unwrap_or(0),
        threads: get(17).parse().unwrap_or(0),
        starttime: get(19).parse().unwrap_or(0),
        vsize: get(20).parse().unwrap_or(0),
        rss_pages: get(21).parse().unwrap_or(0),
    })
}

/// Parse `/proc/<pid>/statm` (`(size, resident, shared, ...)`, in pages).
pub fn parse_statm(text: &str) -> (u64, u64, u64) {
    let mut values = text
        .split_whitespace()
        .map(|token| token.parse::<u64>().unwrap_or(0));
    (
        values.next().unwrap_or(0),
        values.next().unwrap_or(0),
        values.next().unwrap_or(0),
    )
}

/// Parse NUL separated `/proc/<pid>/cmdline`.
pub fn parse_cmdline(text: &str) -> String {
    let parts: Vec<&str> = text.split('\0').filter(|part| !part.is_empty()).collect();
    if parts.is_empty() {
        return String::new();
    }
    parts.join(" ")
}

#[derive(Debug, Clone, Copy)]
struct PrevProc {
    ticks: u64,
    starttime: u64,
}

/// Stateful process table reader.
pub struct ProcReader {
    clock_ticks: u64,
    page_size: u64,
    users: HashMap<u32, String>,
    prev: HashMap<u32, PrevProc>,
    last_wall: f64,
    gpu_cache: HashMap<u32, GpuProcStats>,
    gpu_scan_at: Option<Instant>,
    pub gpu_scan_ms: f64,
    pub gpu_scan_interval: f64,
    engine_totals: HashMap<String, u64>,
    prev_engine_totals: HashMap<String, u64>,
    engine_window: f64,
    pub cmdline_cache: HashMap<u32, String>,
}

impl ProcReader {
    pub fn new(clock_ticks: u64) -> Self {
        ProcReader {
            clock_ticks: clock_ticks.max(1),
            page_size: crate::collect::ttm::page_size(),
            users: load_users(),
            prev: HashMap::new(),
            last_wall: 0.0,
            gpu_cache: HashMap::new(),
            gpu_scan_at: None,
            gpu_scan_ms: 0.0,
            gpu_scan_interval: 2.0,
            engine_totals: HashMap::new(),
            prev_engine_totals: HashMap::new(),
            engine_window: 0.0,
            cmdline_cache: HashMap::new(),
        }
    }

    /// Collect the process table. `wall` is seconds since an arbitrary origin.
    pub fn sample(&mut self, wall: f64, mem_total: u64) -> Vec<ProcInfo> {
        let dt = if self.last_wall > 0.0 {
            (wall - self.last_wall).max(1e-6)
        } else {
            0.0
        };
        self.last_wall = wall;

        if self.gpu_scan_at.map_or(true, |at| at.elapsed().as_secs_f64() >= self.gpu_scan_interval) {
            self.scan_gpu();
        }

        let mut out = Vec::new();
        let mut seen: HashSet<u32> = HashSet::new();
        let Ok(entries) = fs::read_dir("/proc") else {
            return out;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let Ok(pid) = name.parse::<u32>() else { continue };
            let base = Path::new("/proc").join(&name);
            let Some(text) = util::read_string(base.join("stat")) else {
                continue;
            };
            let Some(stat) = parse_stat(&text) else {
                continue;
            };
            seen.insert(pid);
            let (size_pages, resident_pages, shared_pages) = util::read_string(base.join("statm"))
                .map(|text| parse_statm(&text))
                .unwrap_or((0, stat.rss_pages, 0));

            let was_known = self.prev.contains_key(&pid);
            let ticks = stat.utime + stat.stime;
            let cpu = match self.prev.get(&pid) {
                Some(prev)
                    if prev.starttime == stat.starttime && dt > 0.0 =>
                {
                    let delta = ticks.saturating_sub(prev.ticks) as f64;
                    (delta / (dt * self.clock_ticks as f64)) * 100.0
                }
                _ => 0.0,
            };
            self.prev.insert(
                pid,
                PrevProc {
                    ticks,
                    starttime: stat.starttime,
                },
            );

            let rss = resident_pages.max(stat.rss_pages) * self.page_size;
            let virt = if stat.vsize > 0 { stat.vsize } else { size_pages * self.page_size };
            let shared = shared_pages * self.page_size;
            let uid = util::read_string(base.join("status"))
                .and_then(|status| {
                    status.lines().find_map(|line| {
                        line.strip_prefix("Uid:")
                            .and_then(|value| value.split_whitespace().next())
                            .and_then(|value| value.parse::<u32>().ok())
                    })
                })
                .unwrap_or(0);

            // Trust the cached cmdline only for a pid seen in the *previous*
            // sample: a recycled pid must re-read, or it inherits the dead
            // process' command line.
            let cmdline = self
                .cmdline_cache
                .get(&pid)
                .cloned()
                .filter(|_| was_known)
                .unwrap_or_else(|| {
                    let value = util::read_raw(base.join("cmdline"))
                        .map(|text| parse_cmdline(&text))
                        .unwrap_or_default();
                    self.cmdline_cache.insert(pid, value.clone());
                    value
                });

            let gpu = self.gpu_cache.get(&pid).cloned().unwrap_or_default();
            out.push(ProcInfo {
                pid,
                ppid: stat.ppid,
                uid,
                user: self
                    .users
                    .get(&uid)
                    .cloned()
                    .unwrap_or_else(|| uid.to_string()),
                state: stat.state,
                comm: stat.comm.clone(),
                cmdline,
                nice: stat.nice,
                priority: stat.priority,
                threads: stat.threads,
                virt,
                rss,
                shared,
                cpu,
                mem: if mem_total > 0 {
                    (rss as f64 / mem_total as f64) * 100.0
                } else {
                    0.0
                },
                time: ticks as f64 / self.clock_ticks as f64,
                gtt: gpu.gtt,
                gtt_requested: gpu.gtt_requested,
                vram: gpu.vram,
                vram_requested: gpu.vram_requested,
                evicted_vram: gpu.evicted_vram,
                gpu_sys: gpu.sys,
                gpu_clients: gpu.clients,
                depth: 0,
            });
        }
        // Forget processes that disappeared.
        self.prev.retain(|pid, _| seen.contains(pid));
        self.cmdline_cache.retain(|pid, _| seen.contains(pid));
        out
    }

    /// Seconds since the last per-process GPU scan.
    pub fn gpu_scan_age(&self) -> f64 {
        self.gpu_scan_at
            .map_or(f64::INFINITY, |at| at.elapsed().as_secs_f64())
    }

    /// Engine busy ratios (gfx, compute, dec, ...) since the previous scan.
    pub fn engine_ratios(&mut self) -> Vec<(String, f64)> {
        if self.engine_window <= 0.0 {
            return Vec::new();
        }
        let mut out: Vec<(String, f64)> = self
            .engine_totals
            .iter()
            .filter_map(|(engine, total)| {
                let prev = self.prev_engine_totals.get(engine).copied().unwrap_or(0);
                let delta = total.saturating_sub(prev) as f64;
                let ratio = delta / (self.engine_window * 1e9);
                if ratio > 0.0005 {
                    Some((engine.clone(), crate::util::clamp(ratio, 0.0, 8.0)))
                } else {
                    Some((engine.clone(), 0.0))
                }
            })
            .collect();
        out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        out
    }

    /// Walk every process' `/proc/<pid>/fdinfo` looking for DRM clients.
    fn scan_gpu(&mut self) {
        let started = Instant::now();
        let mut cache: HashMap<u32, GpuProcStats> = HashMap::new();
        let mut totals: HashMap<String, u64> = HashMap::new();
        let Ok(entries) = fs::read_dir("/proc") else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let Ok(pid) = name.parse::<u32>() else { continue };
            let base = Path::new("/proc").join(&name);
            let Ok(fds) = fs::read_dir(base.join("fd")) else {
                continue;
            };
            let mut stats = GpuProcStats::default();
            let mut seen_clients: HashSet<(String, u64)> = HashSet::new();
            let mut found = false;
            for fd in fds.flatten() {
                let target = match fs::read_link(fd.path()) {
                    Ok(target) => target.to_string_lossy().to_string(),
                    Err(_) => continue,
                };
                if !target.starts_with("/dev/dri/") {
                    continue;
                }
                let Some(text) = util::read_string(base.join("fdinfo").join(fd.file_name())) else {
                    continue;
                };
                if !text.contains("drm-driver") {
                    continue;
                }
                let info = parse_fdinfo(&text);
                let key = (
                    info.pdev.clone(),
                    info.client_id.unwrap_or_else(|| fd.file_name().to_string_lossy().parse().unwrap_or(0)),
                );
                if !seen_clients.insert(key) {
                    continue;
                }
                found = true;
                stats.clients += 1;
                merge_fdinfo(&mut stats, &info);
                for (engine, ns) in &info.engines {
                    *totals.entry(engine.clone()).or_insert(0) += *ns;
                }
            }
            if found {
                cache.insert(pid, stats);
            }
        }
        match self.gpu_scan_at {
            // First scan: baseline the cumulative counters so the first ratios
            // come out at zero instead of the whole since-boot total.
            None => {
                self.prev_engine_totals = totals.clone();
                self.engine_window = self.gpu_scan_interval.max(0.001);
            }
            Some(at) => {
                self.prev_engine_totals = std::mem::take(&mut self.engine_totals);
                // The window is the wall time between scans, not the scan duration.
                self.engine_window = at.elapsed().as_secs_f64().max(0.001);
            }
        }
        self.engine_totals = totals;
        self.gpu_cache = cache;
        self.gpu_scan_ms = started.elapsed().as_secs_f64() * 1000.0;
        self.gpu_scan_at = Some(Instant::now());
    }
}

/// Map uids to login names from `/etc/passwd` (no libc round trips).
fn load_users() -> HashMap<u32, String> {
    let mut out = HashMap::new();
    if let Some(text) = util::read_string("/etc/passwd") {
        for line in text.lines() {
            let fields: Vec<&str> = line.split(':').collect();
            if fields.len() >= 3 {
                if let Ok(uid) = fields[2].parse::<u32>() {
                    out.entry(uid).or_insert_with(|| fields[0].to_string());
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stat_with_spaces_in_comm() {
        let text = "4242 (my weird proc) S 1 4242 4242 0 -1 4194304 100 0 0 0 500 250 0 0 20 0 8 0 98765 7340032000 512 18446744073709551615 0 0";
        let stat = parse_stat(text).unwrap();
        assert_eq!(stat.pid, 4242);
        assert_eq!(stat.comm, "my weird proc");
        assert_eq!(stat.state, 'S');
        assert_eq!(stat.ppid, 1);
        assert_eq!(stat.utime, 500);
        assert_eq!(stat.stime, 250);
        assert_eq!(stat.priority, 20);
        assert_eq!(stat.nice, 0);
        assert_eq!(stat.threads, 8);
        assert_eq!(stat.vsize, 7340032000);
        assert_eq!(stat.rss_pages, 512);
    }

    #[test]
    fn parses_amdgpu_fdinfo() {
        let text = "\
drm-driver: amdgpu
drm-client-id:  7
drm-pdev:  0000:c5:00.0
pasid:  3
drm-total-vram: 0 KiB
drm-resident-vram:  1024 KiB
drm-total-gtt:  2048 KiB
drm-resident-gtt:  2048 KiB
drm-total-cpu:  512 KiB
drm-resident-cpu:  256 KiB
amd-evicted-vram: 64 KiB
amd-requested-gtt: 2048 KiB
drm-engine-gfx: 123456789 ns
drm-engine-compute: 500 ns
drm-memory-vram:  1024 KiB
";
        let info = parse_fdinfo(text);
        assert_eq!(info.driver, "amdgpu");
        assert_eq!(info.client_id, Some(7));
        assert_eq!(info.pdev, "0000:c5:00.0");
        assert_eq!(info.resident.get("vram"), Some(&(1024 * 1024u64)));
        assert_eq!(info.total.get("gtt"), Some(&(2048 * 1024u64)));
        assert_eq!(info.resident.get("cpu"), Some(&(256 * 1024u64)));
        assert_eq!(info.evicted.get("vram"), Some(&(64 * 1024u64)));
        assert_eq!(info.engines.get("gfx"), Some(&123456789));

        let mut merged = GpuProcStats::default();
        merge_fdinfo(&mut merged, &info);
        assert_eq!(merged.vram, 1024 * 1024);
        assert_eq!(merged.gtt, 2048 * 1024);
        assert_eq!(merged.sys, 256 * 1024);
        assert_eq!(merged.evicted_vram, 64 * 1024);
    }

    #[test]
    fn legacy_memory_keys_are_aliases() {
        let text = "drm-driver: amdgpu\ndrm-memory-gtt: 4096 KiB\n";
        let info = parse_fdinfo(text);
        assert_eq!(info.resident.get("gtt"), Some(&(4096 * 1024u64)));
    }

    #[test]
    fn cmdline_survives_nul_separators() {
        // Regression: read_string() strips NULs, which glued arguments together.
        let raw = util::read_raw("/proc/self/cmdline").unwrap_or_default();
        let parsed = parse_cmdline(&raw);
        assert!(!parsed.contains("\u{0}"), "NUL leaked into cmdline");
        if raw.contains('\u{0}') {
            assert!(
                parsed.split_whitespace().count() >= 1,
                "arguments were not separated: {:?}",
                parsed
            );
        }
    }

    #[test]
    fn parses_cmdline() {
        assert_eq!(parse_cmdline("/usr/bin/foo\0--bar\0baz\0"), "/usr/bin/foo --bar baz");
        assert_eq!(parse_cmdline(""), "");
    }

    #[test]
    fn parses_statm() {
        assert_eq!(parse_statm("100 20 5 1 0 2 0"), (100, 20, 5));
    }
}
