//! One-shot JSON output (`--json`), hand rolled to avoid a serde dependency.

use crate::collect::{ProcInfo, Snapshot};

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn number(value: f64) -> String {
    if !value.is_finite() {
        return "null".to_string();
    }
    format!("{:.4}", value)
}

fn opt_number(value: Option<f64>) -> String {
    match value {
        Some(value) => number(value),
        None => "null".to_string(),
    }
}

fn opt_u64(value: Option<u64>) -> String {
    match value {
        Some(value) => value.to_string(),
        None => "null".to_string(),
    }
}

fn process_json(proc: &ProcInfo) -> String {
    format!(
        "      {{\"pid\": {}, \"ppid\": {}, \"user\": \"{}\", \"comm\": \"{}\", \"cmdline\": \"{}\", \
\"state\": \"{}\", \"cpu_pct\": {}, \"mem_pct\": {}, \"virt\": {}, \"rss\": {}, \"shared\": {}, \
\"cpu_time_s\": {}, \"threads\": {}, \
\"gpu\": {{\"gtt\": {}, \"gtt_requested\": {}, \"vram\": {}, \"vram_requested\": {}, \
\"vram_evicted\": {}, \"system\": {}, \"clients\": {}}}}}",
        proc.pid,
        proc.ppid,
        escape(&proc.user),
        escape(&proc.comm),
        escape(&proc.cmdline),
        proc.state,
        number(proc.cpu),
        number(proc.mem),
        proc.virt,
        proc.rss,
        proc.shared,
        number(proc.time),
        proc.threads,
        proc.gtt,
        proc.gtt_requested,
        proc.vram,
        proc.vram_requested,
        proc.evicted_vram,
        proc.gpu_sys,
        proc.gpu_clients
    )
}

/// Serialise a snapshot, including the process table (capped at `max_procs`).
pub fn snapshot(snap: &Snapshot, max_procs: usize) -> String {
    let mut procs: Vec<&ProcInfo> = snap.procs.iter().collect();
    procs.sort_by(|a, b| {
        b.cpu
            .partial_cmp(&a.cpu)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    procs.truncate(max_procs);

    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(&format!("  \"wall\": {},\n", number(snap.wall)));
    out.push_str("  \"cpu\": {\n");
    out.push_str(&format!("    \"total\": {},\n", number(snap.cpu.total)));
    out.push_str(&format!("    \"user\": {},\n", number(snap.cpu.user)));
    out.push_str(&format!("    \"system\": {},\n", number(snap.cpu.system)));
    out.push_str(&format!("    \"io\": {},\n", number(snap.cpu.io)));
    out.push_str(&format!("    \"steal\": {},\n", number(snap.cpu.steal)));
    out.push_str(&format!(
        "    \"load\": [{}, {}, {}],\n",
        number(snap.cpu.load[0]),
        number(snap.cpu.load[1]),
        number(snap.cpu.load[2])
    ));
    out.push_str(&format!("    \"nproc\": {},\n", snap.cpu.nproc));
    out.push_str(&format!("    \"model\": \"{}\",\n", escape(&snap.cpu.model)));
    out.push_str(&format!("    \"uptime_s\": {},\n", number(snap.cpu.uptime)));
    out.push_str(&format!("    \"temp_c\": {},\n", opt_number(snap.cpu.temp)));
    out.push_str(&format!(
        "    \"freq_mhz_avg\": {},\n",
        opt_number(snap.cpu.avg_freq_mhz())
    ));
    let per_core: Vec<String> = snap.cpu.per_core.iter().map(|value| number(*value)).collect();
    out.push_str(&format!("    \"per_core\": [{}]\n", per_core.join(", ")));
    out.push_str("  },\n");

    out.push_str("  \"gpu\": ");
    match &snap.gpu {
        None => out.push_str("null,\n"),
        Some(gpu) => {
            out.push_str("{\n");
            out.push_str(&format!("    \"card\": \"{}\",\n", escape(&gpu.card)));
            out.push_str(&format!("    \"pci_slot\": \"{}\",\n", escape(&gpu.pci_slot)));
            out.push_str(&format!("    \"pci_id\": \"{}\",\n", escape(&gpu.pci_id)));
            out.push_str(&format!("    \"marketing\": \"{}\",\n", escape(&gpu.marketing)));
            out.push_str(&format!("    \"driver\": \"{}\",\n", escape(&gpu.driver)));
            out.push_str(&format!("    \"busy\": {},\n", opt_number(gpu.busy)));
            out.push_str(&format!("    \"mem_busy\": {},\n", opt_number(gpu.mem_busy)));
            out.push_str(&format!("    \"vram_used\": {},\n", opt_u64(gpu.vram_used)));
            out.push_str(&format!("    \"vram_total\": {},\n", opt_u64(gpu.vram_total)));
            out.push_str(&format!("    \"gtt_used\": {},\n", opt_u64(gpu.gtt_used)));
            out.push_str(&format!("    \"gtt_total\": {},\n", opt_u64(gpu.gtt_total)));
            out.push_str(&format!(
                "    \"vis_vram_used\": {},\n",
                opt_u64(gpu.vis_vram_used)
            ));
            out.push_str(&format!(
                "    \"vis_vram_total\": {},\n",
                opt_u64(gpu.vis_vram_total)
            ));
            out.push_str(&format!("    \"sclk_mhz\": {},\n", opt_number(gpu.sclk)));
            out.push_str(&format!("    \"sclk_min_mhz\": {},\n", opt_number(gpu.sclk_min)));
            out.push_str(&format!("    \"sclk_max_mhz\": {},\n", opt_number(gpu.sclk_max)));
            out.push_str(&format!("    \"mclk_mhz\": {},\n", opt_number(gpu.mclk)));
            out.push_str(&format!("    \"power_w\": {},\n", opt_number(gpu.power_w)));
            out.push_str(&format!("    \"fan_rpm\": {},\n", opt_number(gpu.fan_rpm)));
            let temps: Vec<String> = gpu
                .temps
                .iter()
                .map(|(label, value)| format!("\"{}\": {}", escape(label), number(*value)))
                .collect();
            out.push_str(&format!("    \"temps\": {{{}}}\n", temps.join(", ")));
            out.push_str("  },\n");
        }
    }

    out.push_str("  \"memory\": {\n");
    out.push_str(&format!("    \"total\": {},\n", snap.mem.total));
    out.push_str(&format!("    \"used\": {},\n", snap.mem.used));
    out.push_str(&format!("    \"available\": {},\n", snap.mem.available));
    out.push_str(&format!("    \"free\": {},\n", snap.mem.free));
    out.push_str(&format!("    \"buffers\": {},\n", snap.mem.buffers));
    out.push_str(&format!("    \"cached\": {},\n", snap.mem.cached));
    out.push_str(&format!("    \"shared\": {},\n", snap.mem.shared));
    out.push_str(&format!("    \"kernel\": {},\n", snap.mem.kernel_stack + snap.mem.page_tables));
    out.push_str(&format!("    \"swap_total\": {},\n", snap.mem.swap_total));
    out.push_str(&format!("    \"swap_used\": {},\n", snap.mem.swap_used));
    out.push_str(&format!(
        "    \"gpu_gtt\": {},\n",
        opt_u64(snap.gpu.as_ref().and_then(|g| g.gtt_used))
    ));
    out.push_str(&format!(
        "    \"gpu_vram\": {},\n",
        opt_u64(snap.gpu.as_ref().and_then(|g| g.vram_used))
    ));
    out.push_str(&format!(
        "    \"cpu_only\": {}\n",
        snap.mem
            .used
            .saturating_sub(snap.gpu.as_ref().and_then(|g| g.gtt_used).unwrap_or(0))
    ));
    out.push_str("  },\n");

    out.push_str("  \"ttm\": {\n");
    out.push_str(&format!("    \"available\": {},\n", snap.ttm.available));
    out.push_str(&format!(
        "    \"path\": {},\n",
        snap.ttm
            .path
            .as_ref()
            .map(|path| format!("\"{}\"", escape(path)))
            .unwrap_or_else(|| "null".to_string())
    ));
    out.push_str(&format!("    \"page_size\": {},\n", snap.ttm.page_size));
    for (name, stats) in [("vram_mm", &snap.ttm.vram), ("gtt_mm", &snap.ttm.gtt)] {
        match stats {
            Some(stats) => out.push_str(&format!(
                "    \"{}\": {{\"size\": {}, \"usage\": {}}},\n",
                name,
                opt_u64(stats.size),
                opt_u64(stats.usage)
            )),
            None => out.push_str(&format!("    \"{}\": null,\n", name)),
        }
    }
    let params: Vec<String> = snap
        .ttm
        .params
        .iter()
        .map(|(name, value)| format!("\"{}\": \"{}\"", escape(name), escape(value)))
        .collect();
    out.push_str(&format!("    \"params\": {{{}}}\n", params.join(", ")));
    out.push_str("  },\n");

    let engines: Vec<String> = snap
        .engines
        .iter()
        .map(|(name, value)| format!("\"{}\": {}", escape(name), number(*value)))
        .collect();
    out.push_str(&format!("  \"engines\": {{{}}},\n", engines.join(", ")));

    out.push_str(&format!("  \"hostname\": \"{}\",\n", escape(&snap.hostname)));
    out.push_str("  \"battery\": ");
    match &snap.battery {
        None => out.push_str("null,\n"),
        Some(battery) => out.push_str(&format!(
            "{{\"name\": \"{}\", \"capacity\": {}, \"status\": \"{}\", \"power_w\": {}}},\n",
            escape(&battery.name),
            opt_number(battery.capacity),
            escape(&battery.status),
            opt_number(battery.power_w)
        )),
    }
    let disks: Vec<String> = snap
        .disks
        .iter()
        .map(|disk| {
            format!(
                "{{\"name\": \"{}\", \"model\": \"{}\", \"size\": {}, \"read_bps\": {}, \
\"write_bps\": {}, \"read_iops\": {}, \"write_iops\": {}, \"busy\": {}}}",
                escape(&disk.name),
                escape(&disk.model),
                disk.size,
                number(disk.read_bps),
                number(disk.write_bps),
                number(disk.read_iops),
                number(disk.write_iops),
                number(disk.busy)
            )
        })
        .collect();
    out.push_str(&format!("  \"disks\": [{}],\n", disks.join(", ")));
    let net: Vec<String> = snap
        .net
        .iter()
        .map(|iface| {
            format!(
                "{{\"name\": \"{}\", \"rx_bps\": {}, \"tx_bps\": {}, \"rx_total\": {}, \"tx_total\": {}}}",
                escape(&iface.name),
                number(iface.rx_bps),
                number(iface.tx_bps),
                iface.rx_total,
                iface.tx_total
            )
        })
        .collect();
    out.push_str(&format!("  \"net\": [{}],\n", net.join(", ")));

    out.push_str(&format!("  \"process_count\": {},\n", snap.procs.len()));
    let rows: Vec<String> = procs.into_iter().map(process_json).collect();
    out.push_str("  \"processes\": [\n");
    out.push_str(&rows.join(",\n"));
    out.push_str("\n  ]\n");
    out.push_str("}\n");
    out
}
