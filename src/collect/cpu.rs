//! CPU sampling: `/proc/stat` deltas, cpufreq, hwmon temperatures, topology.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::collect::CpuSample;
use crate::util;

/// Raw jiffy counters from one `/proc/stat` line.
#[derive(Debug, Clone, Copy, Default)]
pub struct CpuTimes {
    pub user: u64,
    pub nice: u64,
    pub system: u64,
    pub idle: u64,
    pub iowait: u64,
    pub irq: u64,
    pub softirq: u64,
    pub steal: u64,
}

impl CpuTimes {
    pub fn parse(fields: &[&str]) -> CpuTimes {
        let get = |i: usize| fields.get(i).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
        CpuTimes {
            user: get(0),
            nice: get(1),
            system: get(2),
            idle: get(3),
            iowait: get(4),
            irq: get(5),
            softirq: get(6),
            steal: get(7),
        }
    }

    pub fn total(&self) -> u64 {
        self.user
            + self.nice
            + self.system
            + self.idle
            + self.iowait
            + self.irq
            + self.softirq
            + self.steal
    }

    pub fn busy(&self) -> u64 {
        self.total().saturating_sub(self.idle + self.iowait)
    }

    /// Busy ratio between two samples; `None` when time did not advance.
    pub fn ratio_since(&self, prev: &CpuTimes) -> Option<f64> {
        let total = self.total().saturating_sub(prev.total()) as f64;
        if total <= 0.0 {
            return Some(0.0);
        }
        let busy = self.busy().saturating_sub(prev.busy()) as f64;
        Some(crate::util::clamp(busy / total, 0.0, 1.0))
    }

    pub fn component_since(&self, prev: &CpuTimes, pick: fn(&CpuTimes) -> u64) -> Option<f64> {
        let total = self.total().saturating_sub(prev.total()) as f64;
        if total <= 0.0 {
            return Some(0.0);
        }
        let delta = pick(self).saturating_sub(pick(prev)) as f64;
        Some(crate::util::clamp(delta / total, 0.0, 1.0))
    }
}

/// Stateful reader that turns `/proc/stat` into ratios.
pub struct CpuReader {
    prev_total: Option<CpuTimes>,
    prev_cores: Vec<CpuTimes>,
    pub model: String,
    pub nproc: usize,
    siblings: Vec<Vec<usize>>,
    hwmon: Vec<(String, PathBuf, Option<f64>)>,
    freq_paths: Vec<Option<PathBuf>>,
    clock_ticks: u64,
}

impl CpuReader {
    pub fn new() -> Self {
        let nproc = num_cpus();
        let model = cpu_model();
        let siblings = read_siblings(nproc);
        let hwmon = discover_hwmon();
        let freq_paths = (0..nproc)
            .map(|core| cpufreq_path(core))
            .collect();
        CpuReader {
            prev_total: None,
            prev_cores: Vec::new(),
            model,
            nproc,
            siblings,
            hwmon,
            freq_paths,
            clock_ticks: 100,
        }
    }

    pub fn clock_ticks(&self) -> u64 {
        self.clock_ticks
    }

    pub fn sample(&mut self) -> CpuSample {
        let stat = util::read_string("/proc/stat").unwrap_or_default();
        let mut sample = CpuSample {
            nproc: self.nproc,
            model: self.model.clone(),
            siblings: self.siblings.clone(),
            per_core: vec![0.0; self.nproc],
            freq_mhz: vec![None; self.nproc],
            ..Default::default()
        };
        sample.uptime = util::read_trim("/proc/uptime")
            .and_then(|s| s.split_whitespace().next().and_then(|v| v.parse().ok()))
            .unwrap_or(0.0);
        if let Some(load) = util::read_string("/proc/loadavg") {
            let parts: Vec<&str> = load.split_whitespace().collect();
            for (slot, value) in sample.load.iter_mut().zip(parts.iter()) {
                *slot = value.parse().unwrap_or(0.0);
            }
        }

        let mut totals = CpuTimes::default();
        let mut cores: Vec<(usize, CpuTimes)> = Vec::with_capacity(self.nproc);
        for line in stat.lines() {
            let mut parts = line.split_whitespace();
            let Some(name) = parts.next() else { continue };
            let fields: Vec<&str> = parts.collect();
            if name == "cpu" {
                totals = CpuTimes::parse(&fields);
            } else if let Some(index) = name.strip_prefix("cpu") {
                if let Ok(index) = index.parse::<usize>() {
                    cores.push((index, CpuTimes::parse(&fields)));
                }
            } else if name == "procs_running" {
                sample.procs_running = fields.first().and_then(|v| v.parse().ok());
            } else if name == "ctxt" {
                sample.context_switches = fields.first().and_then(|v| v.parse().ok());
            }
        }
        sample.procs_total = fs::read_dir("/proc")
            .map(|rd| rd.flatten().filter(|e| e.file_name().to_string_lossy().parse::<u32>().is_ok()).count() as u64)
            .ok();

        // Aggregate ratios.
        if let Some(prev) = &self.prev_total {
            sample.total = totals.ratio_since(prev).unwrap_or(0.0);
            sample.user = totals.component_since(prev, |t| t.user + t.nice).unwrap_or(0.0);
            sample.system = totals.component_since(prev, |t| t.system + t.irq + t.softirq).unwrap_or(0.0);
            sample.io = totals.component_since(prev, |t| t.iowait).unwrap_or(0.0);
            sample.irq = totals.component_since(prev, |t| t.irq + t.softirq).unwrap_or(0.0);
            sample.steal = totals.component_since(prev, |t| t.steal).unwrap_or(0.0);
        }
        self.prev_total = Some(totals);

        // Per core ratios.
        let mut core_times: Vec<CpuTimes> = vec![Default::default(); self.nproc];
        for (index, times) in cores {
            if index < core_times.len() {
                core_times[index] = times;
            }
        }
        if self.prev_cores.len() == core_times.len() {
            for ((index, times), prev) in core_times.iter().enumerate().zip(self.prev_cores.iter()) {
                let _ = index;
                if let Some(ratio) = times.ratio_since(prev) {
                    if index < sample.per_core.len() {
                        sample.per_core[index] = ratio;
                    }
                }
            }
        }
        self.prev_cores = core_times;

        // Frequencies (kHz in sysfs).
        for (core, path) in self.freq_paths.iter().enumerate() {
            if let Some(path) = path {
                if let Some(khz) = util::read_u64(path) {
                    if core < sample.freq_mhz.len() {
                        sample.freq_mhz[core] = Some(khz as f64 / 1000.0);
                    }
                }
            }
        }

        // Temperatures: prefer CPU sensors (k10temp / zenpower / coretemp).
        for (name, path, max) in &self.hwmon {
            if let Some(value) = read_hwmon_temp(path) {
                let lower = name.to_ascii_lowercase();
                if lower.contains("k10temp") || lower.contains("zenpower") || lower.contains("coretemp") {
                    sample.temp = Some(value);
                    sample.temp_max = *max;
                    sample.temp_label = if lower.contains("k10temp") { "Tctl".to_string() } else { name.clone() };
                    break;
                } else if sample.temp.is_none() {
                    sample.temp = Some(value);
                    sample.temp_max = *max;
                    sample.temp_label = name.clone();
                }
            }
        }
        sample
    }
}

fn num_cpus() -> usize {
    if let Some(text) = util::read_string("/proc/cpuinfo") {
        let count = text
            .lines()
            .filter(|line| line.starts_with("processor"))
            .count();
        if count > 0 {
            return count;
        }
    }
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
}

fn cpu_model() -> String {
    util::read_string("/proc/cpuinfo")
        .and_then(|text| {
            text.lines()
                .find(|line| line.starts_with("model name") || line.starts_with("Model"))
                .map(|line| line.split_once(':').map(|(_, v)| v.trim().to_string()).unwrap_or_default())
        })
        .unwrap_or_else(|| "unknown CPU".to_string())
}

/// Parse a sysfs cpu list such as `0,16` or `0-7,16-23`.
pub fn parse_cpu_list(text: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for part in text.trim().split(',') {
        if part.is_empty() {
            continue;
        }
        if let Some((start, end)) = part.split_once('-') {
            if let (Ok(a), Ok(b)) = (start.parse::<usize>(), end.parse::<usize>()) {
                for value in a..=b {
                    out.push(value);
                }
            }
        } else if let Ok(value) = part.parse::<usize>() {
            out.push(value);
        }
    }
    out
}

fn read_siblings(nproc: usize) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut index_of: HashMap<usize, usize> = HashMap::new();
    for core in 0..nproc {
        let path = format!("/sys/devices/system/cpu/cpu{}/topology/thread_siblings_list", core);
        let siblings = util::read_trim(&path)
            .map(|text| parse_cpu_list(&text))
            .unwrap_or_else(|| vec![core]);
        let key = siblings.iter().copied().min().unwrap_or(core);
        if let Some(slot) = index_of.get(&key).copied() {
            if !groups[slot].contains(&core) {
                groups[slot].push(core);
            }
        } else {
            index_of.insert(key, groups.len());
            groups.push(vec![core]);
        }
    }
    groups
}

fn cpufreq_path(core: usize) -> Option<PathBuf> {
    let direct = PathBuf::from(format!(
        "/sys/devices/system/cpu/cpu{}/cpufreq/scaling_cur_freq",
        core
    ));
    if direct.exists() {
        return Some(direct);
    }
    // Fall back to the cpufreq policy that governs this CPU.
    if let Ok(entries) = fs::read_dir("/sys/devices/system/cpu/cpufreq") {
        for entry in entries.flatten() {
            let affected = entry.path().join("affected_cpus");
            if let Some(text) = util::read_trim(&affected) {
                if text
                    .split_whitespace()
                    .any(|value| value.parse::<usize>() == Ok(core))
                {
                    let path = entry.path().join("scaling_cur_freq");
                    if path.exists() {
                        return Some(path);
                    }
                }
            }
        }
    }
    None
}

/// Discover hwmon chips and remember their max temperature when exposed.
fn discover_hwmon() -> Vec<(String, PathBuf, Option<f64>)> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir("/sys/class/hwmon") else {
        return out;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        let Some(name) = util::read_trim(&dir.join("name")) else {
            continue;
        };
        let max = util::read_u64(&dir.join("temp1_max")).map(|v| v as f64 / 1000.0);
        out.push((name, dir, max));
    }
    out
}

fn read_hwmon_temp(dir: &Path) -> Option<f64> {
    for candidate in ["temp1_input", "temp2_input", "temp1_max_input"] {
        if let Some(value) = util::read_u64(&dir.join(candidate)) {
            return Some(value as f64 / 1000.0);
        }
    }
    None
}

/// GPU temperatures come from the amdgpu hwmon chip (`edge` / `junction` / `mem`).
pub fn hwmon_sensor_temps(dir: &Path) -> Vec<(String, f64)> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    let mut inputs: Vec<(u32, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(index) = name.strip_prefix("temp").and_then(|rest| rest.strip_suffix("_input")) {
            if let Ok(index) = index.parse::<u32>() {
                inputs.push((index, entry.path()));
            }
        }
    }
    inputs.sort();
    for (index, path) in inputs {
        let Some(raw) = util::read_u64(&path) else { continue };
        let label_path = dir.join(format!("temp{}_label", index));
        let label = util::read_trim(&label_path).unwrap_or_else(|| match index {
            1 => "edge".to_string(),
            2 => "junction".to_string(),
            3 => "mem".to_string(),
            _ => format!("temp{}", index),
        });
        out.push((label, raw as f64 / 1000.0));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cpu_lists() {
        assert_eq!(parse_cpu_list("0,16"), vec![0, 16]);
        assert_eq!(parse_cpu_list("0-3"), vec![0, 1, 2, 3]);
        assert_eq!(parse_cpu_list("0-1,8-9"), vec![0, 1, 8, 9]);
    }

    #[test]
    fn computes_busy_ratios() {
        let prev = CpuTimes { user: 10, idle: 90, ..Default::default() };
        let now = CpuTimes { user: 60, idle: 140, ..Default::default() };
        let ratio = now.ratio_since(&prev).unwrap();
        assert!((ratio - 0.5).abs() < 1e-9);
    }

    #[test]
    fn parses_stat_lines() {
        let text = "cpu  1 2 3 400 5 0 0 0 0 0\ncpu0 1 0 1 100 0 0 0 0 0 0\ncpu1 0 2 2 300 5 0 0 0 0 0\nprocs_running 3\n";
        let mut totals = CpuTimes::default();
        let mut cores = Vec::new();
        for line in text.lines() {
            let mut parts = line.split_whitespace();
            let name = parts.next().unwrap();
            let fields: Vec<&str> = parts.collect();
            match name {
                "cpu" => totals = CpuTimes::parse(&fields),
                "cpu0" | "cpu1" => cores.push(CpuTimes::parse(&fields)),
                _ => {}
            }
        }
        assert_eq!(totals.user, 1);
        assert_eq!(cores.len(), 2);
        assert_eq!(cores[1].idle, 300);
    }
}
