//! TTM / GTT accounting: `debugfs` resource managers, page pool and module params.
//!
//! Kernel interfaces (verified against torvalds/linux):
//! * `dri/<minor>/amdgpu_vram_mm`, `amdgpu_gtt_mm` — `ttm_resource_manager_debug()`
//!   prints `use_type`, `use_tt`, `size`, `usage`, all sizes in **bytes**.
//! * `dri/<minor>/ttm_page_pool` — `ttm_pool_debugfs()`, one row per caching type
//!   with a count per page order, plus `total nodeN : used of limit`.
//! * `/sys/module/amdgpu/parameters/gttsize` (MiB) and
//!   `/sys/module/ttm/parameters/pages_limit` (pages) bound the GTT domain.

use std::path::{Path, PathBuf};

use crate::collect::{ManStats, TtmPool, TtmSample};
use crate::util;

/// Locate the debugfs `dri/<minor>` directory for a card, if readable.
pub fn find_dri_dir(primary_minor: Option<u32>) -> Option<PathBuf> {
    let root = PathBuf::from("/sys/kernel/debug/dri");
    let entries = std::fs::read_dir(&root).ok()?;
    let mut candidates: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.join("amdgpu_vram_mm").exists() || path.join("amdgpu_gtt_mm").exists() {
            candidates.push(path);
        }
    }
    if candidates.is_empty() {
        return None;
    }
    if let Some(minor) = primary_minor {
        let wanted = root.join(minor.to_string());
        if candidates.contains(&wanted) {
            return Some(wanted);
        }
    }
    candidates.sort();
    candidates.into_iter().next()
}

/// Whether debugfs itself is mounted and reachable (usually needs root).
pub fn debugfs_readable() -> bool {
    Path::new("/sys/kernel/debug").exists() && std::fs::read_dir("/sys/kernel/debug").is_ok()
}

pub fn page_size() -> u64 {
    let size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if size > 0 {
        size as u64
    } else {
        4096
    }
}

/// Build the full TTM view. `dri_dir` is the debugfs directory for the GPU.
pub fn read(dri_dir: Option<&Path>) -> TtmSample {
    let page_size = page_size();
    let mut sample = TtmSample {
        page_size,
        ..Default::default()
    };
    sample.params = module_params();

    let Some(dir) = dri_dir else {
        sample.available = false;
        sample.error = Some(if debugfs_readable() {
            "no amdgpu TTM debugfs under /sys/kernel/debug/dri/*/ (kernel without CONFIG_DEBUG_FS?)".to_string()
        } else {
            "/sys/kernel/debug is not readable — run amdtop with sudo for TTM details".to_string()
        });
        return sample;
    };
    sample.path = Some(dir.display().to_string());

    let vram = read_man(dir.join("amdgpu_vram_mm"));
    let gtt = read_man(dir.join("amdgpu_gtt_mm"));
    sample.vram = vram;
    sample.gtt = gtt;
    sample.pool = read_pool(dir.join("ttm_page_pool"));
    sample.available = sample.vram.is_some() || sample.gtt.is_some() || !sample.pool.rows.is_empty();
    if !sample.available {
        sample.error = Some(format!("could not read TTM files under {}", dir.display()));
    }
    sample
}

fn read_man(path: PathBuf) -> Option<ManStats> {
    let text = util::read_string(&path)?;
    Some(parse_man(&text))
}

/// Parse a `ttm_resource_manager_debug()` dump.
pub fn parse_man(text: &str) -> ManStats {
    let mut stats = ManStats {
        raw: text.trim().to_string(),
        ..Default::default()
    };
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        match key {
            "use_type" => stats.use_type = value.parse().ok(),
            "use_tt" => stats.use_tt = value.parse().ok(),
            // The kernel exports these in bytes (man->size = real_vram_size).
            "size" => stats.size = value.parse().ok(),
            "usage" => stats.usage = value.parse().ok(),
            _ => {}
        }
    }
    stats
}

/// Parse a `ttm_pool_debugfs()` dump.
pub fn parse_pool(text: &str) -> TtmPool {
    let mut pool = TtmPool {
        unused: text.trim() == "unused",
        ..Default::default()
    };
    for line in text.lines() {
        let Some((label, rest)) = line.split_once(':') else {
            continue;
        };
        let label = label.trim().to_string();
        let numbers: Vec<u64> = rest
            .split_whitespace()
            .filter_map(|token| token.parse::<u64>().ok())
            .collect();
        if label.starts_with("total node") {
            let node = label.trim_start_matches("total ").to_string();
            if numbers.len() >= 2 {
                pool.totals.push((node, numbers[0], numbers[1]));
            }
        } else if !numbers.is_empty() {
            pool.rows.push((label, numbers));
        }
    }
    pool
}

fn read_pool(path: PathBuf) -> TtmPool {
    match util::read_string(&path) {
        Some(text) => parse_pool(&text),
        None => TtmPool::default(),
    }
}

/// Relevant module parameters that bound the GTT domain.
pub fn module_params() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let wanted: [(&str, &str); 6] = [
        ("/sys/module/amdgpu/parameters/gttsize", "amdgpu.gttsize (MiB)"),
        ("/sys/module/amdgpu/parameters/gartsize", "amdgpu.gartsize (MiB)"),
        ("/sys/module/amdgpu/parameters/vis_vramlimit", "amdgpu.vis_vramlimit (MiB)"),
        ("/sys/module/ttm/parameters/pages_limit", "ttm.pages_limit (pages)"),
        ("/sys/module/ttm/parameters/page_pool_size", "ttm.page_pool_size (pages)"),
        ("/sys/module/ttm/parameters/dmem_cgroup", "ttm.dmem_cgroup"),
    ];
    for (path, label) in wanted {
        if let Some(value) = util::read_trim(path) {
            out.push((label.to_string(), value));
        }
    }
    out
}

/// GTT ceiling implied by `ttm.pages_limit` (0 means "auto/unlimited").
pub fn pages_limit_bytes(pages_limit: Option<&str>, page_size: u64) -> Option<u64> {
    let pages: i64 = pages_limit?.trim().parse().ok()?;
    if pages <= 0 {
        return None;
    }
    Some(pages as u64 * page_size)
}

/// GTT ceiling implied by `amdgpu.gttsize` (MiB, -1 = auto).
pub fn gttsize_bytes(mib: Option<&str>) -> Option<u64> {
    let value: i64 = mib?.trim().parse().ok()?;
    if value <= 0 {
        return None;
    }
    Some(value as u64 * 1024 * 1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vram_manager_dump() {
        let text = "  use_type: 0\n  use_tt: 0\n  size: 536870912\n  usage: 268435456\n";
        let stats = parse_man(text);
        assert_eq!(stats.size, Some(512 * 1024 * 1024));
        assert_eq!(stats.usage, Some(256 * 1024 * 1024));
        assert!((stats.ratio().unwrap() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn parses_page_pool_dump() {
        let text = "\t --- 0---  --1---  --2---\nDMA wc :        0        0        0\nwc\t:      128       16        0\nuc\t:        4        0        0\n\ntotal node0\t:    40960 of   131072\n";
        let pool = parse_pool(text);
        assert_eq!(pool.rows.len(), 3);
        assert_eq!(pool.rows[0].1, vec![0, 0, 0]);
        assert_eq!(pool.rows[1].0, "wc");
        assert_eq!(pool.rows[1].1, vec![128, 16, 0]);
        assert_eq!(pool.totals.len(), 1);
        assert_eq!(pool.totals[0], ("node0".to_string(), 40960, 131072));
    }

    #[test]
    fn pool_unused_marker() {
        assert!(parse_pool("unused\n").unused);
    }

    #[test]
    fn derives_gt_limits() {
        assert_eq!(pages_limit_bytes(Some("16777216"), 4096), Some(64 * 1024 * 1024 * 1024));
        assert_eq!(pages_limit_bytes(Some("0"), 4096), None);
        assert_eq!(gttsize_bytes(Some("-1")), None);
        assert_eq!(gttsize_bytes(Some("8192")), Some(8 * 1024 * 1024 * 1024));
    }
}
