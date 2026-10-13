//! The real backend: reads the live system through `/proc` and `/sys`.

use std::path::PathBuf;
use std::time::Instant;

use crate::collect::cpu::CpuReader;
use crate::collect::disks::DiskReader;
use crate::collect::net::NetReader;
use crate::collect::procs::ProcReader;
use crate::collect::{gpu, mem, ttm, Backend, Snapshot};
use crate::util;

pub struct LinuxBackend {
    cpu: CpuReader,
    gpu: Option<gpu::GpuDevice>,
    dri: Option<PathBuf>,
    procs: ProcReader,
    disks: DiskReader,
    net: NetReader,
    battery: Option<crate::collect::system::BatteryInfo>,
    hostname: String,
    started: Instant,
    kernel: String,
}

impl LinuxBackend {
    pub fn new() -> Self {
        Self::with_options(false)
    }

    /// `force_gpu_scan` retries discovery even when nothing was found, which is
    /// useful for `--debug` output.
    pub fn with_options(_force_gpu_scan: bool) -> Self {
        let cpu = CpuReader::new();
        let gpu_device = gpu::discover();
        let dri = ttm::find_dri_dir(gpu_device.as_ref().and_then(|g| g.primary_minor));
        let kernel = util::read_trim("/proc/sys/kernel/osrelease").unwrap_or_default();
        LinuxBackend {
            procs: ProcReader::new(cpu.clock_ticks()),
            disks: DiskReader::new(),
            net: NetReader::new(),
            battery: crate::collect::system::read_battery(),
            hostname: crate::collect::system::hostname(),
            cpu,
            dri,
            gpu: gpu_device,
            started: Instant::now(),
            kernel,
        }
    }
}

impl Backend for LinuxBackend {
    fn sample(&mut self) -> Snapshot {
        let wall = self.started.elapsed().as_secs_f64();
        let cpu = self.cpu.sample();
        let mem = mem::read_meminfo();
        let gpu = self.gpu.as_ref().map(gpu::sample);
        let ttm = ttm::read(self.dri.as_deref());
        let procs = self.procs.sample(wall, mem.total);
        let engines = self.procs.engine_ratios();
        let disks = self.disks.sample(wall);
        let net = self.net.sample(wall);
        Snapshot {
            wall,
            disks,
            net,
            battery: self.battery.clone(),
            hostname: self.hostname.clone(),
            cpu,
            gpu,
            mem,
            ttm,
            procs,
            engines,
            gpu_scan_age: self.procs.gpu_scan_age(),
            gpu_scan_ms: self.procs.gpu_scan_ms,
        }
    }

    fn info(&self) -> Vec<(String, String)> {
        let mut out = vec![
            ("kernel".to_string(), self.kernel.clone()),
            ("cpu".to_string(), self.cpu.model.clone()),
            ("logical cpus".to_string(), self.cpu.nproc.to_string()),
            ("clock ticks".to_string(), self.cpu.clock_ticks().to_string()),
            (
                "processes".to_string(),
                format!("{} cached cmdlines", self.procs.cmdline_cache.len()),
            ),
        ];
        match &self.gpu {
            Some(device) => {
                out.push(("card".to_string(), device.card.clone()));
                out.push(("driver".to_string(), device.driver.clone()));
                out.push(("pci id".to_string(), device.pci_id.clone()));
                out.push(("pci slot".to_string(), device.pci_slot.clone()));
                out.push((
                    "marketing".to_string(),
                    gpu::marketing_name(&device.pci_id)
                        .unwrap_or("unknown")
                        .to_string(),
                ));
                out.push((
                    "device path".to_string(),
                    device.device.display().to_string(),
                ));
            }
            None => out.push(("amdgpu".to_string(), "no AMD GPU found".to_string())),
        }
        out.push((
            "ttm debugfs".to_string(),
            self.dri
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "not readable (needs root?)".to_string()),
        ));
        out
    }
}
