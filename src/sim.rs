//! A simulated Strix Halo machine.
//!
//! `--demo` runs the whole UI against this backend so the layout, the memory
//! views and the process table can be exercised (and screenshotted) on any box,
//! including machines without an AMD GPU.

use std::collections::HashMap;

use crate::collect::ProcInfo;
use crate::collect::disks::DiskStats;
use crate::collect::net::NetStats;
use crate::collect::system::BatteryInfo;
use crate::collect::{
    Backend, CpuSample, GpuSample, ManStats, MemSample, Snapshot, TtmPool, TtmSample,
};

const GIB: u64 = 1024 * 1024 * 1024;
const MIB: u64 = 1024 * 1024;

/// Tiny deterministic RNG so demo runs are reproducible.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn unit(&mut self) -> f64 {
        (self.next() % 10_000) as f64 / 10_000.0
    }

    fn range(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * self.unit()
    }
}

/// Static description of one simulated process.
struct SimProc {
    pid: u32,
    ppid: u32,
    user: &'static str,
    comm: &'static str,
    cmdline: &'static str,
    base_cpu: f64,
    rss: u64,
    gtt: u64,
    vram: u64,
    gpu_sys: u64,
    time: f64,
    state: char,
}

const PROCS: &[SimProc] = &[
    SimProc { pid: 1, ppid: 0, user: "root", comm: "systemd", cmdline: "/sbin/init splash", base_cpu: 0.1, rss: 18 * MIB, gtt: 0, vram: 0, gpu_sys: 0, time: 12.4, state: 'S' },
    SimProc { pid: 412, ppid: 1, user: "root", comm: "amdgpu", cmdline: "[kworker/0:1H-kblockd]", base_cpu: 0.0, rss: 0, gtt: 0, vram: 0, gpu_sys: 0, time: 3.1, state: 'I' },
    SimProc { pid: 881, ppid: 1, user: "root", comm: "pipewire", cmdline: "/usr/bin/pipewire", base_cpu: 0.3, rss: 24 * MIB, gtt: 0, vram: 0, gpu_sys: 0, time: 88.2, state: 'S' },
    SimProc { pid: 1204, ppid: 1, user: "gdm", comm: "Xorg", cmdline: "/usr/lib/xorg/Xorg :0 -background none", base_cpu: 1.2, rss: 210 * MIB, gtt: 380 * MIB, vram: 96 * MIB, gpu_sys: 42 * MIB, time: 1042.0, state: 'S' },
    SimProc { pid: 1450, ppid: 1, user: "user", comm: "plasmashell", cmdline: "/usr/bin/plasmashell --no-respawn", base_cpu: 0.8, rss: 420 * MIB, gtt: 512 * MIB, vram: 64 * MIB, gpu_sys: 18 * MIB, time: 812.5, state: 'S' },
    SimProc { pid: 1620, ppid: 1, user: "user", comm: "firefox", cmdline: "/usr/lib/firefox/firefox -contentproc -childID 4", base_cpu: 3.5, rss: 980 * MIB, gtt: 1400 * MIB, vram: 120 * MIB, gpu_sys: 60 * MIB, time: 2210.0, state: 'S' },
    SimProc { pid: 1621, ppid: 1620, user: "user", comm: "firefox", cmdline: "/usr/lib/firefox/firefox -contentproc -childID 5", base_cpu: 6.1, rss: 1240 * MIB, gtt: 900 * MIB, vram: 80 * MIB, gpu_sys: 30 * MIB, time: 1902.0, state: 'R' },
    SimProc { pid: 2201, ppid: 1, user: "user", comm: "llama-server", cmdline: "./llama-server -m llama3-70b-Q4_K_M.gguf --n-gpu-layers 62", base_cpu: 42.0, rss: 6 * GIB, gtt: 41 * GIB, vram: 486 * MIB, gpu_sys: 512 * MIB, time: 4412.0, state: 'R' },
    SimProc { pid: 2455, ppid: 1, user: "user", comm: "python", cmdline: "python train.py --batch 32 --amp", base_cpu: 88.0, rss: 3 * GIB, gtt: 12 * GIB, vram: 210 * MIB, gpu_sys: 128 * MIB, time: 921.3, state: 'R' },
    SimProc { pid: 2456, ppid: 2455, user: "user", comm: "pt_main_thread", cmdline: "python train.py --worker 1", base_cpu: 74.0, rss: 1 * GIB, gtt: 0, vram: 0, gpu_sys: 0, time: 640.0, state: 'R' },
    SimProc { pid: 2900, ppid: 1, user: "user", comm: "cargo", cmdline: "cargo build --release", base_cpu: 120.0, rss: 1 * GIB, gtt: 0, vram: 0, gpu_sys: 0, time: 122.0, state: 'R' },
    SimProc { pid: 2901, ppid: 2900, user: "user", comm: "rustc", cmdline: "rustc -C opt-level=3 src/main.rs", base_cpu: 96.0, rss: 780 * MIB, gtt: 0, vram: 0, gpu_sys: 0, time: 98.0, state: 'R' },
    SimProc { pid: 3100, ppid: 1, user: "messagebus", comm: "dbus-daemon", cmdline: "/usr/bin/dbus-daemon --system", base_cpu: 0.1, rss: 9 * MIB, gtt: 0, vram: 0, gpu_sys: 0, time: 4.2, state: 'S' },
    SimProc { pid: 3311, ppid: 1, user: "user", comm: "code", cmdline: "/usr/share/code/code --unity-launch", base_cpu: 1.4, rss: 620 * MIB, gtt: 260 * MIB, vram: 30 * MIB, gpu_sys: 12 * MIB, time: 302.0, state: 'S' },
    SimProc { pid: 3500, ppid: 1, user: "root", comm: "dockerd", cmdline: "dockerd -H fd://", base_cpu: 0.4, rss: 180 * MIB, gtt: 0, vram: 0, gpu_sys: 0, time: 55.0, state: 'S' },
    SimProc { pid: 3620, ppid: 3500, user: "root", comm: "containerd", cmdline: "containerd --config /etc/containerd/config.toml", base_cpu: 0.2, rss: 62 * MIB, gtt: 0, vram: 0, gpu_sys: 0, time: 21.0, state: 'S' },
    SimProc { pid: 4021, ppid: 1, user: "user", comm: "mpv", cmdline: "mpv --hwdec=vaapi big_buck_bunny.mp4", base_cpu: 4.0, rss: 210 * MIB, gtt: 320 * MIB, vram: 40 * MIB, gpu_sys: 96 * MIB, time: 410.0, state: 'S' },
    SimProc { pid: 4400, ppid: 1, user: "user", comm: "amdtop", cmdline: "./target/release/amdtop", base_cpu: 0.6, rss: 14 * MIB, gtt: 0, vram: 0, gpu_sys: 0, time: 2.0, state: 'S' },
];

pub struct SimBackend {
    rng: Rng,
    tick: u64,
    core_load: Vec<f64>,
    gtt_used: u64,
    vram_used: u64,
    gpu_busy: f64,
    power: f64,
    mem_used: u64,
    gtt_total: u64,
    vram_total: u64,
    mem_total: u64,
}

impl SimBackend {
    pub fn new() -> Self {
        // Strix Halo reference config: 16C/32T, 128 GiB unified LPDDR5X.
        SimBackend {
            rng: Rng(0x5EED_1586_1586_1586),
            tick: 0,
            core_load: vec![0.05; 32],
            gtt_used: 12 * GIB,
            vram_used: 420 * MIB,
            gpu_busy: 0.12,
            power: 18.0,
            mem_used: 34 * GIB,
            gtt_total: 64 * GIB,
            vram_total: 512 * MIB,
            mem_total: 126 * GIB,
        }
    }

    fn wobble(&mut self, value: f64, amount: f64, low: f64, high: f64) -> f64 {
        let next = value + self.rng.range(-amount, amount);
        crate::util::clamp(next, low, high)
    }
}

impl Backend for SimBackend {
    fn sample(&mut self) -> Snapshot {
        self.tick += 1;
        let wave = ((self.tick as f64) / 12.0).sin() * 0.5 + 0.5;

        // CPU: a few busy cores (the rustc/cargo pair) plus light background.
        let mut per_core = Vec::with_capacity(32);
        for core in 0..32 {
            let busy_core = matches!(core, 0 | 1 | 2 | 3 | 16 | 17 | 18 | 19);
            let target = if busy_core {
                0.55 + 0.4 * wave
            } else {
                0.02 + 0.06 * self.rng.unit()
            };
            self.core_load[core] = crate::util::clamp(
                self.core_load[core] * 0.6 + target * 0.4,
                0.0,
                1.0,
            );
            per_core.push(self.core_load[core]);
        }
        let total = per_core.iter().sum::<f64>() / per_core.len() as f64;

        // GPU: inference job keeps the compute queue busy in bursts.
        let burst = if self.tick % 40 < 22 { 1.0 } else { 0.15 };
        self.gpu_busy = crate::util::clamp(
            self.gpu_busy * 0.5 + (0.15 + 0.8 * burst * wave) * 0.5,
            0.0,
            1.0,
        );
        self.power = self.wobble(12.0 + self.gpu_busy * 55.0 + total * 20.0, 2.0, 6.0, 120.0);
        self.vram_used = crate::util::clamp(
            self.vram_used as f64 + self.rng.range(-8.0, 8.0) * MIB as f64,
            320.0 * MIB as f64,
            (self.vram_total - 8 * MIB) as f64,
        ) as u64;
        self.gtt_used = crate::util::clamp(
            self.gtt_used as f64 + self.rng.range(-160.0, 170.0) * MIB as f64,
            8.0 * GIB as f64,
            (self.gtt_total * 3 / 4) as f64,
        ) as u64;
        self.mem_used = crate::util::clamp(
            self.mem_used as f64 + self.rng.range(-220.0, 220.0) * MIB as f64 + (self.gtt_used as f64 - 12.0 * GIB as f64) * 0.02,
            20.0 * GIB as f64,
            (self.mem_total - 6 * GIB) as f64,
        ) as u64;

        let cached = 22 * GIB;
        let mem = MemSample {
            total: self.mem_total,
            free: self.mem_total.saturating_sub(self.mem_used + cached),
            available: self.mem_total.saturating_sub(self.mem_used).saturating_sub(2 * GIB),
            used: self.mem_used,
            buffers: 420 * MIB,
            cached: cached + 1_800 * MIB,
            sreclaimable: 1_800 * MIB,
            shmem: 2 * GIB,
            shared: 2 * GIB,
            swap_total: 16 * GIB,
            swap_free: 15 * GIB + 700 * MIB,
            swap_used: 1 * GIB - 700 * MIB,
            kernel_stack: 96 * MIB,
            page_tables: 640 * MIB,
            huge_pages: 0,
            dirty: 12 * MIB,
            writeback: 0,
        };

        let gpu = GpuSample {
            card: "card0".to_string(),
            pci_slot: "0000:c5:00.0".to_string(),
            pci_id: "1002:1586".to_string(),
            marketing: "Radeon 8060S (gfx1151, Strix Halo)".to_string(),
            driver: "amdgpu".to_string(),
            busy: Some(self.gpu_busy),
            mem_busy: Some(crate::util::clamp(self.gpu_busy * 0.7, 0.0, 1.0)),
            vram_total: Some(self.vram_total),
            vram_used: Some(self.vram_used),
            vis_vram_total: Some(self.vram_total),
            vis_vram_used: Some(self.vram_used / 2),
            gtt_total: Some(self.gtt_total),
            gtt_used: Some(self.gtt_used),
            sclk: Some(600.0 + self.gpu_busy * 2100.0),
            sclk_min: Some(213.0),
            sclk_max: Some(2900.0),
            mclk: Some(1600.0 + self.gpu_busy * 600.0),
            mclk_min: Some(420.0),
            mclk_max: Some(2400.0),
            temps: vec![
                ("edge".to_string(), 38.0 + self.gpu_busy * 34.0),
                ("junction".to_string(), 42.0 + self.gpu_busy * 44.0),
                ("mem".to_string(), 40.0 + self.gpu_busy * 22.0),
            ],
            power_w: Some(self.power),
            power_cap: Some(120.0),
            fan_rpm: Some(1200.0 + self.gpu_busy * 1600.0),
            fan_percent: Some(crate::util::clamp(0.2 + self.gpu_busy * 0.6, 0.0, 1.0)),
            extra_mem: vec![("cpu_access_total".to_string(), Some(0), Some(0))],
        };

        let cpu = CpuSample {
            total,
            user: total * 0.7,
            system: total * 0.25,
            nice: 0.0,
            io: total * 0.01,
            irq: total * 0.04,
            steal: 0.0,
            per_core,
            freq_mhz: (0..32)
                .map(|core| {
                    let load = self.core_load[core];
                    Some(545.0 + load * 2400.0 + self.rng.range(-30.0, 30.0))
                })
                .collect(),
            temp: Some(41.0 + total * 42.0),
            temp_max: Some(100.0),
            temp_label: "Tctl".to_string(),
            load: [total * 32.0 * 0.9, total * 32.0 * 0.8, total * 32.0 * 0.7],
            nproc: 32,
            model: "AMD Ryzen AI Max+ 395 w/ Radeon 8060S".to_string(),
            uptime: 3.0 * 86400.0 + 4.0 * 3600.0 + (self.tick as f64) * 2.0,
            siblings: (0..16).map(|core| vec![core, core + 16]).collect(),
            procs_running: Some(4 + (total * 8.0) as u64),
            procs_total: Some(PROCS.len() as u64 + 240),
            context_switches: Some(1_200_000 + self.tick * 14_000),
        };

        let procs: Vec<ProcInfo> = PROCS
            .iter()
            .map(|proc| {
                let cpu_pct = crate::util::clamp(
                    proc.base_cpu * (0.7 + 0.6 * self.rng.unit()),
                    0.0,
                    1600.0,
                );
                ProcInfo {
                    pid: proc.pid,
                    ppid: proc.ppid,
                    uid: if proc.user == "root" { 0 } else { 1000 },
                    user: proc.user.to_string(),
                    state: proc.state,
                    comm: proc.comm.to_string(),
                    cmdline: proc.cmdline.to_string(),
                    nice: 0,
                    priority: 20,
                    threads: 1 + (proc.base_cpu as u32 / 4).min(24),
                    virt: proc.rss * 6 + 200 * MIB,
                    rss: proc.rss,
                    shared: proc.rss / 4,
                    cpu: cpu_pct,
                    mem: (proc.rss as f64 / self.mem_total as f64) * 100.0,
                    time: proc.time + (self.tick as f64) * (cpu_pct / 100.0) * 2.0,
                    gtt: scale(proc.gtt, self.gtt_used, 58 * GIB),
                    gtt_requested: scale(proc.gtt, self.gtt_used, 58 * GIB),
                    vram: scale(proc.vram, self.vram_used, 480 * MIB),
                    vram_requested: scale(proc.vram, self.vram_used, 480 * MIB),
                    evicted_vram: if proc.gtt > 4 * GIB { 256 * MIB } else { 0 },
                    gpu_sys: proc.gpu_sys,
                    gpu_clients: if proc.gtt + proc.vram + proc.gpu_sys > 0 { 1 } else { 0 },
                    depth: 0,
                }
            })
            .collect();

        let engines = vec![
            ("gfx".to_string(), self.gpu_busy * 0.55),
            ("compute".to_string(), self.gpu_busy),
            ("video_decode".to_string(), 0.04),
            ("jpeg0".to_string(), 0.0),
        ];

        let ttm = TtmSample {
            available: true,
            error: None,
            path: Some("/sys/kernel/debug/dri/0 (simulated)".to_string()),
            vram: Some(ManStats {
                use_type: Some(1),
                use_tt: Some(0),
                size: Some(self.vram_total),
                usage: Some(self.vram_used),
                raw: String::new(),
            }),
            gtt: Some(ManStats {
                use_type: Some(2),
                use_tt: Some(1),
                size: Some(self.gtt_total),
                usage: Some(self.gtt_used),
                raw: String::new(),
            }),
            pool: TtmPool {
                rows: vec![
                    ("DMA wc".to_string(), vec![1024, 128, 8, 0]),
                    ("DMA uc".to_string(), vec![256, 16, 0, 0]),
                    ("wc".to_string(), vec![4096, 512, 32, 0]),
                    ("uc".to_string(), vec![128, 8, 0, 0]),
                ],
                totals: vec![("node0".to_string(), self.gtt_used / 4096, self.gtt_total / 4096)],
                unused: false,
            },
            params: vec![
                ("amdgpu.gttsize (MiB)".to_string(), "-1".to_string()),
                ("amdgpu.gartsize (MiB)".to_string(), "-1".to_string()),
                ("ttm.pages_limit (pages)".to_string(), (self.gtt_total / 4096).to_string()),
                ("ttm.page_pool_size (pages)".to_string(), "8192".to_string()),
            ],
            page_size: 4096,
        };

        let mut engine_map: HashMap<String, f64> = HashMap::new();
        for (name, ratio) in &engines {
            engine_map.insert(name.clone(), *ratio);
        }

        // A fast NVMe and a wifi link, so the disk/net boxes have something to show.
        let nvme_read = 40.0 * MIB as f64 * self.rng.range(0.2, 6.0);
        let nvme_write = 20.0 * MIB as f64 * self.rng.range(0.1, 9.0);
        let disks = vec![DiskStats {
            name: "nvme0n1".to_string(),
            model: "Samsung SSD 990 PRO 2TB".to_string(),
            read_bps: nvme_read,
            write_bps: nvme_write,
            read_iops: nvme_read / 65536.0,
            write_iops: nvme_write / 65536.0,
            busy: crate::util::clamp((nvme_read + nvme_write) / (1200.0 * MIB as f64), 0.0, 1.0),
            size: 2_000_398_934_016,
        }];
        let net_rx = 900.0 * 1024.0 * self.rng.range(0.05, 11.0);
        let net_tx = 300.0 * 1024.0 * self.rng.range(0.05, 6.0);
        let net = vec![NetStats {
            name: "wlan0".to_string(),
            rx_bps: net_rx,
            tx_bps: net_tx,
            rx_total: 812 * GIB,
            tx_total: 190 * GIB,
        }];
        let battery = Some(BatteryInfo {
            name: "BAT0".to_string(),
            capacity: Some(crate::util::clamp(96.0 - (self.tick as f64 / 900.0), 5.0, 100.0)),
            status: "Discharging".to_string(),
            power_w: Some(self.power * 0.22 + 6.0),
            energy_wh: Some(72.0),
            energy_full_wh: Some(75.0),
        });

        Snapshot {
            wall: self.tick as f64 * 2.0,
            disks,
            net,
            battery,
            hostname: "strix-halo".to_string(),
            cpu,
            gpu: Some(gpu),
            mem,
            ttm,
            procs,
            engines,
            gpu_scan_age: 0.4,
            gpu_scan_ms: 3.2,
        }
    }

    fn info(&self) -> Vec<(String, String)> {
        vec![
            ("backend".to_string(), "simulated Strix Halo (Ryzen AI Max+ 395)".to_string()),
            ("memory".to_string(), "126 GiB unified LPDDR5X".to_string()),
            ("gpu".to_string(), "Radeon 8060S, 512 MiB VRAM carveout".to_string()),
            ("gtt".to_string(), "64 GiB limit".to_string()),
        ]
    }

    fn is_sim(&self) -> bool {
        true
    }
}

/// Scale one process' simulated allocation with the machine total.
fn scale(base: u64, current_total: u64, base_total: u64) -> u64 {
    if base_total == 0 || base == 0 {
        return 0;
    }
    (base as f64 * (current_total as f64 / base_total as f64)) as u64
}
