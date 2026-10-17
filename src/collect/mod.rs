//! Data model, history rings and the sampling loop.
//!
//! A [`Backend`] produces [`Snapshot`]s; the [`Sampler`] owns the backend and
//! keeps the ring buffers that feed the graphs.

#![allow(dead_code)] // the collectors mirror every kernel counter, not all are shown yet

pub mod cpu;
pub mod disks;
pub mod gpu;
pub mod linux;
pub mod mem;
pub mod net;
pub mod npu;
pub mod procs;
pub mod system;
pub mod ttm;

use std::collections::VecDeque;

/// One fixed size ring buffer of samples.
#[derive(Debug, Clone)]
pub struct Ring {
    data: VecDeque<f64>,
    cap: usize,
}

impl Ring {
    pub fn new(cap: usize) -> Self {
        Ring {
            data: VecDeque::with_capacity(cap),
            cap: cap.max(2),
        }
    }

    pub fn push(&mut self, value: f64) {
        if self.data.len() == self.cap {
            self.data.pop_front();
        }
        self.data.push_back(value);
    }

    pub fn values(&self) -> Vec<f64> {
        self.data.iter().copied().collect()
    }

    pub fn last(&self) -> Option<f64> {
        self.data.back().copied()
    }

    pub fn max(&self) -> Option<f64> {
        self.data.iter().copied().fold(None, |acc, v| Some(acc.map_or(v, |a: f64| a.max(v))))
    }

    pub fn mean(&self) -> Option<f64> {
        if self.data.is_empty() {
            return None;
        }
        Some(self.data.iter().sum::<f64>() / self.data.len() as f64)
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

impl Default for Ring {
    fn default() -> Self {
        Ring::new(512)
    }
}

/// All time series the UI graphs.
#[derive(Debug, Clone, Default)]
pub struct History {
    pub cpu: Ring,
    pub cpu_user: Ring,
    pub cpu_system: Ring,
    pub per_core: Vec<Ring>,
    pub cpu_temp: Ring,
    pub cpu_freq: Ring,
    pub gpu: Ring,
    pub npu: Ring,
    pub npu_power: Ring,
    pub npu_clock: Ring,
    pub gpu_mem_busy: Ring,
    pub gpu_temp: Ring,
    pub gpu_power: Ring,
    pub gpu_clock: Ring,
    pub vram: Ring,
    pub gtt: Ring,
    pub mem_used: Ring,
    pub swap_used: Ring,
    pub net_rx: Ring,
    pub net_tx: Ring,
    pub disk_read: Ring,
    pub disk_write: Ring,
}

impl History {
    pub fn new(cap: usize, cores: usize) -> Self {
        History {
            cpu: Ring::new(cap),
            cpu_user: Ring::new(cap),
            cpu_system: Ring::new(cap),
            per_core: (0..cores).map(|_| Ring::new(cap)).collect(),
            cpu_temp: Ring::new(cap),
            cpu_freq: Ring::new(cap),
            gpu: Ring::new(cap),
            npu: Ring::new(cap),
            npu_power: Ring::new(cap),
            npu_clock: Ring::new(cap),
            gpu_mem_busy: Ring::new(cap),
            gpu_temp: Ring::new(cap),
            gpu_power: Ring::new(cap),
            gpu_clock: Ring::new(cap),
            vram: Ring::new(cap),
            gtt: Ring::new(cap),
            mem_used: Ring::new(cap),
            swap_used: Ring::new(cap),
            net_rx: Ring::new(cap),
            net_tx: Ring::new(cap),
            disk_read: Ring::new(cap),
            disk_write: Ring::new(cap),
        }
    }

    pub fn ensure_cores(&mut self, cores: usize) {
        while self.per_core.len() < cores {
            self.per_core.push(Ring::new(self.cpu.cap));
        }
    }

    pub fn push(&mut self, snap: &Snapshot) {
        self.cpu.push(snap.cpu.total);
        self.cpu_user.push(snap.cpu.user);
        self.cpu_system.push(snap.cpu.system);
        self.ensure_cores(snap.cpu.per_core.len());
        for (ring, value) in self.per_core.iter_mut().zip(snap.cpu.per_core.iter()) {
            ring.push(*value);
        }
        if let Some(temp) = snap.cpu.temp {
            self.cpu_temp.push(temp);
        }
        if let Some(freq) = snap.cpu.avg_freq_mhz() {
            self.cpu_freq.push(freq);
        }
        if let Some(gpu) = &snap.gpu {
            self.gpu.push(gpu.busy.unwrap_or(0.0));
            self.gpu_mem_busy.push(gpu.mem_busy.unwrap_or(0.0));
            if let Some((_, temp)) = gpu.hottest() {
                self.gpu_temp.push(temp);
            }
            if let Some(power) = gpu.power_w {
                self.gpu_power.push(power);
            }
            if let Some(clock) = gpu.sclk {
                self.gpu_clock.push(clock);
            }
            self.vram.push(gpu.vram_used.unwrap_or(0) as f64);
            self.gtt.push(gpu.gtt_used.unwrap_or(0) as f64);
        }
        if let Some(npu) = &snap.npu {
            self.npu.push(npu.busy.or_else(|| npu.cols_ratio()).unwrap_or(0.0));
            if let Some(power) = npu.power_w {
                self.npu_power.push(power);
            }
            if let Some(clock) = npu.npu_mhz {
                self.npu_clock.push(clock);
            }
        }
        self.mem_used.push(snap.mem.used as f64);
        self.swap_used.push(snap.mem.swap_used as f64);
        let net = snap.net.first();
        self.net_rx.push(net.map(|n| n.rx_bps).unwrap_or(0.0));
        self.net_tx.push(net.map(|n| n.tx_bps).unwrap_or(0.0));
        self.disk_read
            .push(snap.disks.iter().map(|d| d.read_bps).sum());
        self.disk_write
            .push(snap.disks.iter().map(|d| d.write_bps).sum());
    }
}

/// CPU side of a snapshot.
#[derive(Debug, Clone, Default)]
pub struct CpuSample {
    pub total: f64,
    pub user: f64,
    pub system: f64,
    pub nice: f64,
    pub io: f64,
    pub irq: f64,
    pub steal: f64,
    pub per_core: Vec<f64>,
    pub freq_mhz: Vec<Option<f64>>,
    pub temp: Option<f64>,
    pub temp_max: Option<f64>,
    pub temp_label: String,
    pub load: [f64; 3],
    pub nproc: usize,
    pub model: String,
    pub uptime: f64,
    /// Logical CPUs grouped by physical core (SMT siblings).
    pub siblings: Vec<Vec<usize>>,
    pub procs_running: Option<u64>,
    pub procs_total: Option<u64>,
    pub context_switches: Option<u64>,
}

impl CpuSample {
    pub fn avg_freq_mhz(&self) -> Option<f64> {
        let values: Vec<f64> = self.freq_mhz.iter().flatten().copied().collect();
        if values.is_empty() {
            None
        } else {
            Some(values.iter().sum::<f64>() / values.len() as f64)
        }
    }

    pub fn max_freq_mhz(&self) -> Option<f64> {
        self.freq_mhz.iter().flatten().copied().fold(None, |a: Option<f64>, v| {
            Some(a.map_or(v, |x: f64| x.max(v)))
        })
    }
}

/// GPU side of a snapshot (amdgpu sysfs).
#[derive(Debug, Clone, Default)]
pub struct GpuSample {
    pub card: String,
    pub pci_slot: String,
    pub pci_id: String,
    pub marketing: String,
    pub driver: String,
    pub busy: Option<f64>,
    pub mem_busy: Option<f64>,
    pub vram_total: Option<u64>,
    pub vram_used: Option<u64>,
    pub vis_vram_total: Option<u64>,
    pub vis_vram_used: Option<u64>,
    pub gtt_total: Option<u64>,
    pub gtt_used: Option<u64>,
    pub sclk: Option<f64>,
    pub sclk_min: Option<f64>,
    pub sclk_max: Option<f64>,
    pub mclk: Option<f64>,
    pub mclk_min: Option<f64>,
    pub mclk_max: Option<f64>,
    pub temps: Vec<(String, f64)>,
    pub power_w: Option<f64>,
    pub power_cap: Option<f64>,
    pub fan_rpm: Option<f64>,
    pub fan_percent: Option<f64>,
    /// Anything else the driver exposes as `mem_info_*`.
    pub extra_mem: Vec<(String, Option<u64>, Option<u64>)>,
}

impl GpuSample {
    pub fn hottest(&self) -> Option<(String, f64)> {
        self.temps
            .iter()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(label, value)| (label.clone(), *value))
    }

    pub fn vram_ratio(&self) -> Option<f64> {
        crate::util::ratio(self.vram_used, self.vram_total)
    }

    pub fn gtt_ratio(&self) -> Option<f64> {
        crate::util::ratio(self.gtt_used, self.gtt_total)
    }
}

/// One AIE tile group (`core`, `mem` or `shim`) from `QUERY_AIE_METADATA`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NpuTile {
    pub rows: u16,
    pub row_start: u16,
    pub dma_channels: u16,
    pub locks: u16,
    pub events: u16,
}

impl NpuTile {
    pub fn describe(&self) -> String {
        format!(
            "{} rows @{} · {} dma · {} locks",
            self.rows, self.row_start, self.dma_channels, self.locks
        )
    }
}

/// One NPU hardware context (a process's queue on the array).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NpuContext {
    /// Owning process; 0 for kernel/internal contexts.
    pub pid: i64,
    pub ctx_id: u32,
    pub start_col: u32,
    pub num_col: u32,
    /// `0` idle, `1` active (driver `AMDXDNA_HWCTX_STATE_*`).
    pub state: u32,
    /// Device heap resident for this context, in bytes (system RAM).
    pub heap: u64,
    pub submissions: u64,
    pub completions: u64,
    pub errors: u64,
    pub preemptions: u64,
    pub migrations: u64,
    pub suspensions: u64,
    pub fps: u32,
    pub gops: u32,
    pub dma_bandwidth: u32,
    pub latency: u32,
    pub frame_exec_time: u32,
}

impl NpuContext {
    pub fn active(&self) -> bool {
        self.state == 1
    }

    pub fn column_span(&self) -> String {
        if self.num_col <= 1 {
            format!("{}", self.start_col)
        } else {
            format!("{}-{}", self.start_col, self.start_col + self.num_col - 1)
        }
    }
}

/// NPU side of a snapshot (`amdxdna` + the XDNA query ioctl).
#[derive(Debug, Clone, Default)]
pub struct NpuSample {
    /// `vbnv`, e.g. `RyzenAI-npu5`.
    pub name: String,
    pub bdf: String,
    pub driver: String,
    pub node: String,
    /// Firmware version as the driver reports it, e.g. `1.1.2.65`.
    pub fw: String,
    /// A device was found (it may still be unopenable).
    pub available: bool,
    /// First query/setup error, if any. The rest of the sample may still be valid.
    pub error: Option<String>,

    pub cols: Option<u32>,
    pub rows: Option<u32>,
    pub col_size: Option<u32>,
    pub aie_version: Option<(u32, u32)>,
    pub tile_core: Option<NpuTile>,
    pub tile_mem: Option<NpuTile>,
    pub tile_shim: Option<NpuTile>,

    pub npu_mhz: Option<f64>,
    pub h_mhz: Option<f64>,
    pub npu_max_mhz: Option<f64>,
    pub tops_max: Option<u32>,
    pub tops_curr: Option<u32>,
    pub task_max: Option<u32>,
    pub task_curr: Option<u32>,
    pub power_w: Option<f64>,
    pub power_mode: Option<String>,

    /// Per-column utilization, 0.0..=1.0. Empty when the PMF sensor query is
    /// unavailable (NPU metrics disabled in the kernel).
    pub col_busy: Vec<f64>,
    /// Hottest column, the headline "NPU busy" number.
    pub busy: Option<f64>,
    /// Columns currently assigned to a hardware context (bitmap popcount).
    pub cols_used: Option<u32>,
    /// Raw column bitmap behind `cols_used`.
    pub cols_bitmap: Option<u32>,
    pub sensors_ok: bool,
    pub contexts: Vec<NpuContext>,
}

impl NpuSample {
    pub fn label(&self) -> &str {
        if self.name.is_empty() {
            "AMD NPU"
        } else {
            &self.name
        }
    }

    pub fn used_cols(&self) -> u32 {
        self.cols_used.unwrap_or(0)
    }

    pub fn cols_ratio(&self) -> Option<f64> {
        match (self.cols_used, self.cols) {
            (Some(used), Some(total)) if total > 0 => {
                Some(crate::util::clamp(used as f64 / total as f64, 0.0, 1.0))
            }
            _ => None,
        }
    }

    pub fn top_clock(&self) -> Option<f64> {
        self.npu_mhz.or(self.h_mhz)
    }
}

/// System RAM (`/proc/meminfo`), all values in bytes.
#[derive(Debug, Clone, Default)]
pub struct MemSample {
    pub total: u64,
    pub free: u64,
    pub available: u64,
    pub used: u64,
    pub buffers: u64,
    pub cached: u64,
    pub sreclaimable: u64,
    pub shmem: u64,
    pub shared: u64,
    pub swap_total: u64,
    pub swap_free: u64,
    pub swap_used: u64,
    pub kernel_stack: u64,
    pub page_tables: u64,
    pub huge_pages: u64,
    pub dirty: u64,
    pub writeback: u64,
}

impl MemSample {
    pub fn used_ratio(&self) -> Option<f64> {
        crate::util::ratio(Some(self.used), Some(self.total))
    }
    pub fn swap_ratio(&self) -> Option<f64> {
        crate::util::ratio(Some(self.swap_used), Some(self.swap_total))
    }
}

/// One `ttm_resource_manager` debugfs dump (`amdgpu_vram_mm` / `amdgpu_gtt_mm`).
#[derive(Debug, Clone, Default)]
pub struct ManStats {
    pub use_type: Option<i64>,
    pub use_tt: Option<i64>,
    /// Bytes, as exported by the kernel.
    pub size: Option<u64>,
    pub usage: Option<u64>,
    pub raw: String,
}

impl ManStats {
    pub fn ratio(&self) -> Option<f64> {
        crate::util::ratio(self.usage, self.size)
    }
}

/// `ttm_page_pool` debugfs dump.
#[derive(Debug, Clone, Default)]
pub struct TtmPool {
    pub rows: Vec<(String, Vec<u64>)>,
    pub totals: Vec<(String, u64, u64)>,
    pub unused: bool,
}

/// Everything the TTM view needs, including why it may be unavailable.
#[derive(Debug, Clone, Default)]
pub struct TtmSample {
    pub available: bool,
    pub error: Option<String>,
    pub path: Option<String>,
    pub vram: Option<ManStats>,
    pub gtt: Option<ManStats>,
    pub pool: TtmPool,
    /// `amdgpu.gttsize`, `ttm.pages_limit`, ... as `(name, value)` pairs.
    pub params: Vec<(String, String)>,
    pub page_size: u64,
}

/// A process row, `top` fields plus amdgpu per-client memory.
#[derive(Debug, Clone, Default)]
pub struct ProcInfo {
    pub pid: u32,
    pub ppid: u32,
    pub uid: u32,
    pub user: String,
    pub state: char,
    pub comm: String,
    pub cmdline: String,
    pub nice: i32,
    pub priority: i32,
    pub threads: u32,
    pub virt: u64,
    pub rss: u64,
    pub shared: u64,
    pub cpu: f64,
    pub mem: f64,
    pub time: f64,
    /// Resident GTT (system RAM pinned for the GPU) in bytes.
    pub gtt: u64,
    /// Requested GTT (including non instantiated) in bytes.
    pub gtt_requested: u64,
    /// Resident VRAM in bytes.
    pub vram: u64,
    pub vram_requested: u64,
    /// Buffers evicted from VRAM (spilled into GTT) in bytes.
    pub evicted_vram: u64,
    /// `drm-total-cpu`: buffers the GPU reads straight from system RAM.
    pub gpu_sys: u64,
    /// Distinct DRM clients (deduplicated file descriptors).
    pub gpu_clients: usize,
    /// Set by the UI when rendering the process tree.
    pub depth: usize,
}

impl ProcInfo {
    pub fn has_gpu(&self) -> bool {
        self.gpu_clients > 0
    }
    pub fn gpu_total(&self) -> u64 {
        self.gtt + self.vram + self.gpu_sys
    }
}

/// A full sample of the machine.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub wall: f64,
    pub cpu: CpuSample,
    pub gpu: Option<GpuSample>,
    pub npu: Option<NpuSample>,
    pub mem: MemSample,
    pub ttm: TtmSample,
    pub procs: Vec<ProcInfo>,
    /// Aggregate per-engine GPU busy ratios, e.g. `("gfx", 0.42)`.
    pub disks: Vec<disks::DiskStats>,
    pub net: Vec<net::NetStats>,
    pub battery: Option<system::BatteryInfo>,
    pub hostname: String,
    pub engines: Vec<(String, f64)>,
    /// Seconds since the per-process GPU scan last ran.
    pub gpu_scan_age: f64,
    pub gpu_scan_ms: f64,
}

impl Snapshot {
    pub fn gpu_name(&self) -> &str {
        self.gpu
            .as_ref()
            .map(|g| if g.marketing.is_empty() { g.pci_id.as_str() } else { g.marketing.as_str() })
            .unwrap_or("no amdgpu device")
    }
}

/// Something that can produce snapshots (real hardware or the simulator).
pub trait Backend {
    fn sample(&mut self) -> Snapshot;
    /// `(label, value)` pairs describing the backend, shown in `--debug`.
    fn info(&self) -> Vec<(String, String)>;
    fn is_sim(&self) -> bool {
        false
    }
}

/// Drives a backend and maintains the graphs' history.
pub struct Sampler<B: Backend> {
    pub backend: B,
    pub history: History,
    pub last: Snapshot,
    pub ticks: u64,
}

impl<B: Backend> Sampler<B> {
    pub fn new(mut backend: B, history_cap: usize) -> Self {
        let first = backend.sample();
        let cores = first.cpu.per_core.len().max(1);
        let mut history = History::new(history_cap, cores);
        history.push(&first);
        Sampler {
            backend,
            history,
            last: first,
            ticks: 1,
        }
    }

    pub fn tick(&mut self) {
        let snapshot = self.backend.sample();
        self.history.push(&snapshot);
        self.last = snapshot;
        self.ticks += 1;
    }
}
