//! amdxdna discovery and the XDNA query ioctl.
//!
//! The live NPU counters are not in sysfs. They come from
//! `DRM_IOCTL_AMDXDNA_GET_INFO` on `/dev/accel/accelN`, the same interface
//! `xrt-smi` uses. Everything is best effort: a kernel built without AMD PMF
//! still reports metadata, clocks, TOPS and per-context usage, just not the
//! power/column-utilization sensors.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};

use crate::collect::{NpuContext, NpuSample, NpuTile};
use crate::util;

/// `DRM_IOWR(DRM_COMMAND_BASE + DRM_AMDXDNA_GET_INFO, struct drm_amdxdna_get_info)`.
const IOCTL_GET_INFO: libc::c_ulong = iowr(b'd', 0x47, 16);

const QUERY_AIE_STATUS: u32 = 0;
const QUERY_AIE_METADATA: u32 = 1;
const QUERY_CLOCK_METADATA: u32 = 3;
const QUERY_SENSORS: u32 = 4;
const QUERY_HW_CONTEXTS: u32 = 5;
const QUERY_FIRMWARE_VERSION: u32 = 8;
const GET_POWER_MODE: u32 = 9;
const QUERY_RESOURCE_INFO: u32 = 12;

const SENSOR_SIZE: usize = 168;
const CTX_SIZE: usize = 144;
const MAX_SENSORS: usize = 16;
const MAX_CONTEXTS: usize = 64;

const fn iowr(ty: u8, nr: u32, size: u32) -> libc::c_ulong {
    ((3u64 << 30) | ((size as u64) << 16) | ((ty as u64) << 8) | nr as u64) as libc::c_ulong
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct GetInfo {
    param: u32,
    buffer_size: u32,
    buffer: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct AieStatus {
    buffer: u64,
    buffer_size: u32,
    /// Bitmap of columns assigned to a hardware context.
    cols_filled: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct RawTile {
    row_count: u16,
    row_start: u16,
    dma_channel_count: u16,
    lock_count: u16,
    event_reg_count: u16,
    pad: [u16; 3],
}

impl RawTile {
    fn to_tile(self) -> NpuTile {
        NpuTile {
            rows: self.row_count,
            row_start: self.row_start,
            dma_channels: self.dma_channel_count,
            locks: self.lock_count,
            events: self.event_reg_count,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct AieMetadata {
    col_size: u32,
    cols: u16,
    rows: u16,
    version_major: u32,
    version_minor: u32,
    core: RawTile,
    mem: RawTile,
    shim: RawTile,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Clock {
    name: [u8; 16],
    freq_mhz: u32,
    pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ClockMeta {
    mp_npu: Clock,
    h: Clock,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct FwVersion {
    major: u32,
    minor: u32,
    patch: u32,
    build: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ResourceInfo {
    npu_clk_max: u64,
    npu_tops_max: u64,
    npu_task_max: u64,
    npu_tops_curr: u64,
    npu_task_curr: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Sensor {
    label: [u8; 64],
    input: u32,
    max: u32,
    average: u32,
    highest: u32,
    status: [u8; 64],
    units: [u8; 16],
    unitm: i8,
    sensor_type: u8,
    pad: [u8; 6],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct HwCtx {
    context_id: u32,
    start_col: u32,
    num_col: u32,
    hwctx_id: u32,
    pid: i64,
    command_submissions: u64,
    command_completions: u64,
    migrations: u64,
    preemptions: u64,
    errors: u64,
    priority: u64,
    heap_usage: u64,
    suspensions: u64,
    state: u32,
    pasid: u32,
    gops: u32,
    fps: u32,
    dma_bandwidth: u32,
    latency: u32,
    frame_exec_time: u32,
    txn_op_idx: u32,
    ctx_pc: u32,
    fatal_error_type: u32,
    fatal_error_exception_type: u32,
    fatal_error_exception_pc: u32,
    fatal_error_app_module: u32,
    pad: u32,
}

/// One discovered amdxdna accel node.
#[derive(Debug, Clone)]
pub struct NpuDevice {
    pub accel: String,
    pub name: String,
    pub bdf: String,
    pub driver: String,
    pub fw: String,
    pub node: PathBuf,
}

/// Find the first amdxdna accel device under `/sys/class/accel`.
pub fn discover() -> Option<NpuDevice> {
    let entries = fs::read_dir("/sys/class/accel").ok()?;
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            name.starts_with("accel").then_some(name)
        })
        .collect();
    names.sort();
    for accel in names {
        let sysfs = Path::new("/sys/class/accel").join(&accel);
        let device = sysfs.join("device");
        let uevent = util::read_string(device.join("uevent")).unwrap_or_default();
        let driver = field(&uevent, "DRIVER=").unwrap_or_default();
        if driver != "amdxdna" {
            continue;
        }
        let bdf = field(&uevent, "PCI_SLOT_NAME=").unwrap_or_default();
        let name = util::read_trim(device.join("vbnv"))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "AMD NPU".to_string());
        let fw = util::read_trim(device.join("fw_version")).unwrap_or_default();
        let node = Path::new("/dev/accel").join(&accel);
        return Some(NpuDevice {
            accel,
            name,
            bdf,
            driver,
            fw,
            node,
        });
    }
    None
}

fn field(text: &str, key: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.strip_prefix(key).map(|value| value.trim().to_string()))
}

fn query_raw(fd: RawFd, param: u32, ptr: *mut u8, size: u32) -> io::Result<u32> {
    let mut info = GetInfo {
        param,
        buffer_size: size,
        buffer: ptr as u64,
    };
    let rc = unsafe { libc::ioctl(fd, IOCTL_GET_INFO, &mut info as *mut GetInfo) };
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(info.buffer_size)
    }
}

fn query<T>(fd: RawFd, param: u32, out: &mut T) -> io::Result<u32> {
    query_raw(
        fd,
        param,
        out as *mut T as *mut u8,
        std::mem::size_of::<T>() as u32,
    )
}

fn note(sample: &mut NpuSample, what: &str, error: io::Error) {
    if sample.error.is_none() {
        sample.error = Some(format!("{}: {}", what, error));
    }
}

/// Owns the accel fd and caches the immutable parts of the device description.
pub struct NpuReader {
    dev: NpuDevice,
    file: Option<File>,
    open_error: Option<String>,
    metadata: Option<AieMetadata>,
    firmware: Option<FwVersion>,
}

impl NpuReader {
    pub fn new() -> Option<Self> {
        let dev = discover()?;
        let (file, open_error) = match OpenOptions::new().read(true).write(true).open(&dev.node) {
            Ok(file) => (Some(file), None),
            Err(error) => {
                let error = format!("open {}: {}", dev.node.display(), error);
                match OpenOptions::new().read(true).open(&dev.node) {
                    Ok(file) => (Some(file), Some(error)),
                    Err(_) => (None, Some(error)),
                }
            }
        };
        let mut reader = NpuReader {
            dev,
            file,
            open_error,
            metadata: None,
            firmware: None,
        };
        reader.load_static();
        Some(reader)
    }

    fn load_static(&mut self) {
        let Some(fd) = self.file.as_ref().map(|file| file.as_raw_fd()) else {
            return;
        };
        let mut metadata = AieMetadata::default();
        if query(fd, QUERY_AIE_METADATA, &mut metadata).is_ok() {
            self.metadata = Some(metadata);
        }
        let mut firmware = FwVersion::default();
        if query(fd, QUERY_FIRMWARE_VERSION, &mut firmware).is_ok() {
            self.firmware = Some(firmware);
        }
    }

    /// Static description only; used by `--debug` before the first sample.
    pub fn peek(&self) -> NpuSample {
        self.base_sample()
    }

    fn base_sample(&self) -> NpuSample {
        let mut sample = NpuSample {
            name: self.dev.name.clone(),
            bdf: self.dev.bdf.clone(),
            driver: self.dev.driver.clone(),
            node: self.dev.node.display().to_string(),
            fw: self.dev.fw.clone(),
            available: true,
            ..Default::default()
        };
        if let Some(firmware) = self.firmware {
            sample.fw = format!(
                "{}.{}.{}.{}",
                firmware.major, firmware.minor, firmware.patch, firmware.build
            );
        }
        if let Some(metadata) = self.metadata {
            sample.cols = Some(metadata.cols as u32);
            sample.rows = Some(metadata.rows as u32);
            sample.col_size = Some(metadata.col_size);
            sample.aie_version = Some((metadata.version_major, metadata.version_minor));
            sample.tile_core = Some(metadata.core.to_tile());
            sample.tile_mem = Some(metadata.mem.to_tile());
            sample.tile_shim = Some(metadata.shim.to_tile());
        }
        sample
    }

    pub fn sample(&mut self) -> NpuSample {
        let mut sample = self.base_sample();
        let Some(fd) = self.file.as_ref().map(|file| file.as_raw_fd()) else {
            sample.error = self.open_error.clone();
            return sample;
        };

        let mut clocks = ClockMeta::default();
        match query(fd, QUERY_CLOCK_METADATA, &mut clocks) {
            Ok(_) => {
                sample.npu_mhz = Some(clocks.mp_npu.freq_mhz as f64);
                sample.h_mhz = Some(clocks.h.freq_mhz as f64);
            }
            Err(error) => note(&mut sample, "clocks", error),
        }

        let mut resource = ResourceInfo::default();
        if query(fd, QUERY_RESOURCE_INFO, &mut resource).is_ok() {
            sample.npu_max_mhz = Some(resource.npu_clk_max as f64);
            sample.tops_max = Some(resource.npu_tops_max as u32);
            sample.tops_curr = Some(resource.npu_tops_curr as u32);
            sample.task_max = Some(resource.npu_task_max as u32);
            sample.task_curr = Some(resource.npu_task_curr as u32);
        }

        let mut mode = [0u8; 8];
        if query_raw(fd, GET_POWER_MODE, mode.as_mut_ptr(), mode.len() as u32).is_ok() {
            sample.power_mode = Some(power_mode_name(mode[0]).to_string());
        }

        let mut sensors = vec![0u8; SENSOR_SIZE * MAX_SENSORS];
        match query_raw(
            fd,
            QUERY_SENSORS,
            sensors.as_mut_ptr(),
            sensors.len() as u32,
        ) {
            Ok(used) => {
                let count = (used as usize / SENSOR_SIZE).min(MAX_SENSORS);
                parse_sensors(&sensors[..count * SENSOR_SIZE], &mut sample);
                sample.sensors_ok = true;
            }
            Err(error) => note(&mut sample, "sensors", error),
        }

        let mut status = AieStatus::default();
        let mut status_data = [0u8; 16];
        status.buffer = status_data.as_mut_ptr() as u64;
        status.buffer_size = status_data.len() as u32;
        if query(fd, QUERY_AIE_STATUS, &mut status).is_ok() {
            sample.cols_used = Some(status.cols_filled.count_ones());
            sample.cols_bitmap = Some(status.cols_filled);
        }

        let mut contexts = vec![0u8; CTX_SIZE * MAX_CONTEXTS];
        match query_raw(
            fd,
            QUERY_HW_CONTEXTS,
            contexts.as_mut_ptr(),
            contexts.len() as u32,
        ) {
            Ok(used) => {
                let count = if used == 0 {
                    0
                } else {
                    (used as usize / CTX_SIZE).min(MAX_CONTEXTS)
                };
                sample.contexts = parse_contexts(&contexts[..count * CTX_SIZE]);
            }
            Err(error) => note(&mut sample, "contexts", error),
        }

        sample
            .contexts
            .sort_by_key(|context| (context.pid <= 0, context.pid));
        sample
    }
}

fn parse_sensors(buf: &[u8], sample: &mut NpuSample) {
    for chunk in buf.chunks_exact(SENSOR_SIZE) {
        // The driver's buffer is only byte aligned.
        let sensor: Sensor = unsafe { std::ptr::read_unaligned(chunk.as_ptr() as *const Sensor) };
        let units = cstr(&sensor.units);
        let scale = 10f64.powi(sensor.unitm as i32);
        let value = sensor.input as f64 * scale;
        match sensor.sensor_type {
            0 => {
                // Current drivers report mW with unitm = -3; guard against a
                // firmware that forgets the exponent.
                sample.power_w = Some(if units.eq_ignore_ascii_case("mW") && sensor.unitm == 0 {
                    value / 1000.0
                } else {
                    value
                });
            }
            1 => {
                if units == "%" || units.is_empty() {
                    sample.col_busy.push((value / 100.0).clamp(0.0, 1.0));
                }
            }
            _ => {}
        }
    }
    sample.busy = sample.col_busy.iter().copied().reduce(f64::max);
}

fn parse_contexts(buf: &[u8]) -> Vec<NpuContext> {
    let mut out = Vec::new();
    for chunk in buf.chunks_exact(CTX_SIZE) {
        let raw: HwCtx = unsafe { std::ptr::read_unaligned(chunk.as_ptr() as *const HwCtx) };
        if raw.context_id == 0 && raw.pid == 0 {
            continue;
        }
        out.push(NpuContext {
            pid: raw.pid,
            ctx_id: raw.context_id,
            start_col: raw.start_col,
            num_col: raw.num_col,
            state: raw.state,
            heap: raw.heap_usage,
            submissions: raw.command_submissions,
            completions: raw.command_completions,
            errors: raw.errors,
            preemptions: raw.preemptions,
            migrations: raw.migrations,
            suspensions: raw.suspensions,
            fps: raw.fps,
            gops: raw.gops,
            dma_bandwidth: raw.dma_bandwidth,
            latency: raw.latency,
            frame_exec_time: raw.frame_exec_time,
        });
    }
    out
}

fn cstr(bytes: &[u8]) -> String {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_string()
}

fn power_mode_name(mode: u8) -> &'static str {
    match mode {
        0 => "default",
        1 => "low",
        2 => "medium",
        3 => "high",
        4 => "turbo",
        _ => "?",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put_u32(buf: &mut [u8], offset: usize, value: u32) {
        buf[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
    }

    fn put_u64(buf: &mut [u8], offset: usize, value: u64) {
        buf[offset..offset + 8].copy_from_slice(&value.to_ne_bytes());
    }

    #[test]
    fn ioctl_number_matches_uapi() {
        assert_eq!(IOCTL_GET_INFO, 0xC010_6447);
    }

    #[test]
    fn parses_power_and_column_sensors() {
        let mut buf = vec![0u8; SENSOR_SIZE * 2];
        buf[..7].copy_from_slice(b"Total P");
        put_u32(&mut buf, 64, 9_400);
        buf[144..146].copy_from_slice(b"mW");
        buf[160] = (-3i8) as u8;
        buf[161] = 0;

        let off = SENSOR_SIZE;
        buf[off..off + 8].copy_from_slice(b"Column 0");
        put_u32(&mut buf, off + 64, 42);
        buf[off + 144..off + 145].copy_from_slice(b"%");
        buf[off + 161] = 1;

        let mut sample = NpuSample::default();
        parse_sensors(&buf, &mut sample);
        assert_eq!(sample.power_w, Some(9.4));
        assert_eq!(sample.col_busy, vec![0.42]);
        assert_eq!(sample.busy, Some(0.42));
    }

    #[test]
    fn busiest_column_wins_the_headline() {
        let mut buf = vec![0u8; SENSOR_SIZE * 3];
        for (index, value) in [10u32, 80, 25].iter().enumerate() {
            let off = index * SENSOR_SIZE;
            put_u32(&mut buf, off + 64, *value);
            buf[off + 161] = 1;
        }
        let mut sample = NpuSample::default();
        parse_sensors(&buf, &mut sample);
        assert_eq!(sample.busy, Some(0.8));
    }

    #[test]
    fn parses_hardware_contexts_and_skips_empty_entries() {
        let mut buf = vec![0u8; CTX_SIZE * 2];
        put_u32(&mut buf, 0, 3);
        put_u32(&mut buf, 4, 4);
        put_u32(&mut buf, 8, 4);
        put_u64(&mut buf, 16, 2201);
        put_u64(&mut buf, 24, 17);
        put_u64(&mut buf, 32, 16);
        put_u64(&mut buf, 56, 2);
        put_u64(&mut buf, 72, 512 * 1024 * 1024);
        put_u32(&mut buf, 88, 1);
        put_u32(&mut buf, 100, 38);

        let contexts = parse_contexts(&buf);
        assert_eq!(contexts.len(), 1);
        let context = &contexts[0];
        assert_eq!(context.pid, 2201);
        assert_eq!(context.ctx_id, 3);
        assert_eq!(context.column_span(), "4-7");
        assert_eq!(context.heap, 512 * 1024 * 1024);
        assert!(context.active());
        assert_eq!(context.fps, 38);
    }
}
