//! System memory from `/proc/meminfo`.

use crate::collect::MemSample;
use crate::util;

const KIB: u64 = 1024;

pub fn read_meminfo() -> MemSample {
    let text = util::read_string("/proc/meminfo").unwrap_or_default();
    from_meminfo(&text)
}

/// Parse meminfo text; exposed for tests. Values are converted to bytes.
pub fn from_meminfo(text: &str) -> MemSample {
    let map: std::collections::HashMap<String, u64> = util::proc_key_values(text)
        .into_iter()
        .map(|(key, value)| (key, value * KIB))
        .collect();
    let get = |key: &str| map.get(key).copied().unwrap_or(0);
    let total = get("MemTotal");
    let free = get("MemFree");
    let available = map.get("MemAvailable").copied().unwrap_or(free);
    // "used" follows htop/top: everything the kernel cannot hand to a process.
    let used = total.saturating_sub(available);
    let swap_total = get("SwapTotal");
    let swap_free = get("SwapFree");
    MemSample {
        total,
        free,
        available,
        used,
        buffers: get("Buffers"),
        cached: get("Cached") + get("SReclaimable"),
        sreclaimable: get("SReclaimable"),
        shmem: get("Shmem"),
        shared: get("Shmem"),
        swap_total,
        swap_free,
        swap_used: swap_total.saturating_sub(swap_free),
        kernel_stack: get("KernelStack"),
        page_tables: get("PageTables"),
        huge_pages: get("HugePages_Total") * get("Hugepagesize"),
        dirty: get("Dirty"),
        writeback: get("Writeback"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
MemTotal:       16087544 kB
MemFree:         2297628 kB
MemAvailable:   12045452 kB
Buffers:          100408 kB
Cached:          9963944 kB
SwapTotal:        524284 kB
SwapFree:         524284 kB
Shmem:            123456 kB
SReclaimable:      54321 kB
";

    #[test]
    fn parses_meminfo() {
        let sample = from_meminfo(SAMPLE);
        assert_eq!(sample.total, 16087544 * KIB);
        assert_eq!(sample.used, (16087544 - 12045452) * KIB);
        assert_eq!(sample.cached, (9963944 + 54321) * KIB);
        assert_eq!(sample.swap_used, 0);
        assert_eq!(sample.shared, 123456 * KIB);
    }

    #[test]
    fn handles_missing_meminfo() {
        let sample = from_meminfo("");
        assert_eq!(sample.total, 0);
        assert_eq!(sample.used_ratio(), None);
    }
}
