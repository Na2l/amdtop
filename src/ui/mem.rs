//! Memory panels. Four views, because on Strix Halo "how much memory is in use"
//! only makes sense once you separate CPU RAM, VRAM and GTT:
//!
//! * `Cpu`      — what the kernel hands to processes (`/proc/meminfo`)
//! * `Gpu`      — VRAM carveout + GTT, with per-process totals
//! * `Ttm`      — kernel pool internals from debugfs (needs root)
//! * `Unified`  — one physical pool, split between CPU and GPU

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::collect::{History, Snapshot};
use crate::theme::Gradient;
use crate::theme::Theme;
use crate::util;

use super::widgets;
use super::{App, MemView};

/// btop-style memory box: one labelled bar per metric, GTT among them.
pub fn btop_box(area: Rect, buf: &mut Buffer, snap: &Snapshot, hist: &History, theme: &Theme) {
    let mem = &snap.mem;
    let gtt = snap.gpu.as_ref().and_then(|g| g.gtt_used).unwrap_or(0);
    let vram = snap.gpu.as_ref().and_then(|g| g.vram_used).unwrap_or(0);
    let title = format!("mem · {}", util::human_bytes(mem.total));
    let inner = widgets::panel(theme.block(&title, crate::theme::MEM_BOX), area, buf);
    if inner.width < 14 || inner.height == 0 {
        return;
    }

    // GTT sits third so it survives shrinking; it is the whole point here.
    let rows: [(&str, u64, u64, Gradient); 6] = [
        ("Total:", mem.total, mem.total, Gradient::Free),
        ("Used:", mem.used, mem.total, Gradient::Used),
        ("GTT:", gtt, mem.total, Gradient::Download),
        ("Avail:", mem.available, mem.total, Gradient::Available),
        ("Cached:", mem.cached, mem.total, Gradient::Cached),
        ("Free:", mem.free, mem.total, Gradient::Free),
    ];
    let rows_len = rows.len();
    let label_width = 9u16;
    let value_width = 11u16;
    let pct_width = 5u16;
    let bar_width = inner
        .width
        .saturating_sub(label_width + value_width + pct_width + 2)
        .max(3);

    let mut graph_height = inner.height.saturating_sub(rows_len as u16);
    if graph_height < 3 {
        graph_height = 0;
    }
    let usable = inner.height.saturating_sub(graph_height) as usize;
    for (row, (label, value, total, kind)) in rows.iter().enumerate().take(usable) {
        let y = inner.y + row as u16;
        buf.set_line(
            inner.x,
            y,
            &Line::from(Span::styled(
                util::fit(label, label_width as usize),
                Style::default().fg(crate::theme::DIM),
            )),
            label_width,
        );
        // "Total" has no meaningful ratio, so it gets a full bar.
        let ratio = if *label == "Total:" {
            Some(1.0)
        } else {
            util::ratio(Some(*value), Some(*total))
        };
        // The right-hand text is drawn by the meter so it inherits the
        // metric's gradient (used=red, cached=cyan, free=green, ...).
        widgets::meter(
            Rect {
                x: inner.x + label_width,
                y,
                width: bar_width + 1 + value_width + pct_width,
                height: 1,
            },
            buf,
            ratio,
            "",
            &format!(
                "{:>10} {:>3.0}%",
                util::human_bytes(*value),
                ratio.unwrap_or(0.0) * 100.0
            ),
            theme,
            *kind,
        );
    }
    if graph_height >= 1 && inner.height > 1 {
        let graph = Rect {
            x: inner.x,
            y: inner.bottom() - graph_height as u16,
            width: inner.width,
            height: graph_height as u16,
        };
        widgets::graph(
            graph,
            buf,
            &hist.mem_used.values(),
            mem.total as f64,
            theme,
            Gradient::Cpu,
        );
    }
    let _ = vram;
}

/// Full MEM tab, dispatching on the selected view.
pub fn full(area: Rect, buf: &mut Buffer, app: &App, snap: &Snapshot, hist: &History, theme: &Theme) {
    match app.mem_view {
        MemView::Cpu => view_cpu(area, buf, snap, hist, theme),
        MemView::Gpu => view_gpu(area, buf, snap, hist, theme),
        MemView::Ttm => view_ttm(area, buf, snap, theme),
        MemView::Unified => view_unified(area, buf, snap, hist, theme),
    }
}

/// CPU view: classic `meminfo` breakdown.
fn view_cpu(area: Rect, buf: &mut Buffer, snap: &Snapshot, hist: &History, theme: &Theme) {
    let mem = &snap.mem;
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);
    let left = widgets::panel(theme.block("process RAM (meminfo)", crate::theme::MEM_BOX), columns[0], buf);
    let gtt = snap.gpu.as_ref().and_then(|g| g.gtt_used).unwrap_or(0);
    let rows: [(&str, u64, u64, Gradient); 8] = [
        ("used", mem.used, mem.total, Gradient::Cpu),
        ("cpu-only", mem.used.saturating_sub(gtt), mem.total, Gradient::Cpu),
        ("cache", mem.cached, mem.total, Gradient::Cached),
        ("buffers", mem.buffers, mem.total, Gradient::Cached),
        ("shared", mem.shared, mem.total, Gradient::Download),
        ("kernel", mem.kernel_stack + mem.page_tables, mem.total, Gradient::Free),
        ("free", mem.free, mem.total, Gradient::Free),
        ("available", mem.available, mem.total, Gradient::Available),
    ];
    for (row, (label, value, total, kind)) in rows.iter().enumerate() {
        let y = left.y + row as u16;
        if y >= left.bottom() {
            break;
        }
        widgets::meter(
            Rect {
                x: left.x,
                y,
                width: left.width,
                height: 1,
            },
            buf,
            util::ratio(Some(*value), Some(*total)),
            label,
            &format!("{} / {}", util::human_bytes(*value), util::human_bytes(*total)),
            theme,
            *kind,
        );
    }
    if left.height as usize > rows.len() + 1 {
        let note = Rect {
            x: left.x,
            y: left.y + rows.len() as u16 + 1,
            width: left.width,
            height: 1,
        };
        if note.y < left.bottom() {
            buf.set_line(
                note.x,
                note.y,
                &Line::from(Span::styled(
                    "cpu-only = used − GTT (GTT pages live inside used)",
                    Style::default().fg(crate::theme::DIM),
                )),
                left.width,
            );
        }
    }

    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(columns[1]);
    let graph_inner = widgets::panel(theme.block("used history", crate::theme::MEM_BOX), right[0], buf);
    if graph_inner.width > 2 && graph_inner.height > 1 {
        widgets::graph(
            graph_inner,
            buf,
            &hist.mem_used.values(),
            mem.total as f64,
            theme,
            Gradient::Cpu,
        );
    }
    let stats_inner = widgets::panel(theme.block("swap / details", crate::theme::MEM_BOX), right[1], buf);
    let mut lines = vec![
        widgets::kv("total", util::human_bytes(mem.total), theme),
        widgets::kv("used", util::human_bytes(mem.used), theme),
        widgets::kv("available", util::human_bytes(mem.available), theme),
        widgets::kv("dirty", util::human_bytes(mem.dirty), theme),
        widgets::kv_styled(
            "swap",
            format!(
                "{} / {} ({:.1}%)",
                util::human_bytes(mem.swap_used),
                util::human_bytes(mem.swap_total),
                mem.swap_ratio().unwrap_or(0.0) * 100.0
            ),
            Style::default().fg(if mem.swap_used > 0 {
                crate::theme::WARN
            } else {
                crate::theme::STRONG
            }),
            theme,
        ),
        widgets::kv("slab rec", util::human_bytes(mem.sreclaimable), theme),
        widgets::kv("hugepages", util::human_bytes(mem.huge_pages), theme),
    ];
    if let Some(gpu) = &snap.gpu {
        lines.push(widgets::kv_styled(
            "GTT in use",
            util::human_bytes(gpu.gtt_used.unwrap_or(0)),
            Style::default().fg(crate::theme::GTT_ACCENT),
            theme,
        ));
    }
    widgets::lines(stats_inner, buf, lines);
}

/// GPU view: VRAM carveout and GTT, cross-checked against per-process totals.
fn view_gpu(area: Rect, buf: &mut Buffer, snap: &Snapshot, hist: &History, theme: &Theme) {
    let Some(gpu) = &snap.gpu else {
        let inner = widgets::panel(theme.block("GPU memory", crate::theme::GPU_BOX), area, buf);
        widgets::lines(
            inner,
            buf,
            vec![Line::from(Span::styled(
                "no amdgpu device found",
                Style::default().fg(crate::theme::WARN),
            ))],
        );
        return;
    };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(6), Constraint::Length(6), Constraint::Min(4)])
        .split(area);

    let vram_inner = widgets::panel(theme.block("VRAM (BIOS carveout of the same LPDDR5X)", crate::theme::GPU_BOX), rows[0], buf);
    big_gauge(vram_inner, buf, "VRAM", gpu.vram_used, gpu.vram_total, Gradient::Available, hist, |h| h.vram.values(), theme);

    let gtt_inner = widgets::panel(theme.block("GTT (system RAM pinned for the GPU)", crate::theme::GPU_BOX), rows[1], buf);
    big_gauge(gtt_inner, buf, "GTT", gpu.gtt_used, gpu.gtt_total, Gradient::Download, hist, |h| h.gtt.values(), theme);

    let bottom = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[2]);
    let procs_inner = widgets::panel(theme.block("per-process GPU memory (fdinfo)", crate::theme::GPU_BOX), bottom[0], buf);
    draw_process_totals(procs_inner, buf, snap, theme);
    let notes_inner = widgets::panel(theme.block("notes", crate::theme::GPU_BOX), bottom[1], buf);
    let mut lines = vec![
        Line::from(Span::styled(
            "Strix Halo has no discrete VRAM: the iGPU uses one LPDDR5X pool.",
            Style::default().fg(crate::theme::TEXT),
        )),
        Line::from(Span::styled(
            format!(
                "  VRAM  = fixed carveout ({}) visible as BAR",
                util::human_bytes(gpu.vram_total.unwrap_or(0))
            ),
            Style::default().fg(crate::theme::DIM),
        )),
        Line::from(Span::styled(
            format!(
                "  GTT   = pageable system RAM, limit {}",
                util::human_bytes(gpu.gtt_total.unwrap_or(0))
            ),
            Style::default().fg(crate::theme::DIM),
        )),
        Line::from(Span::styled(
            "  GTT pages are counted inside meminfo `used` too.",
            Style::default().fg(crate::theme::DIM),
        )),
    ];
    if let Some(vis) = gpu.vis_vram_total {
        lines.push(widgets::kv(
            "visible",
            format!(
                "{} / {}",
                util::human_bytes(gpu.vis_vram_used.unwrap_or(0)),
                util::human_bytes(vis)
            ),
            theme,
        ));
    }
    for (name, used, total) in &gpu.extra_mem {
        lines.push(widgets::kv(name, util::human_bytes(used.unwrap_or(0)), theme));
        let _ = total;
    }
    widgets::lines(notes_inner, buf, lines);
}

fn big_gauge<F: Fn(&History) -> Vec<f64>>(
    area: Rect,
    buf: &mut Buffer,
    label: &str,
    used: Option<u64>,
    total: Option<u64>,
    kind: Gradient,
    hist: &History,
    series: F,
    theme: &Theme,
) {
    if area.width < 12 || area.height < 3 {
        return;
    }
    let ratio = util::ratio(used, total);
    widgets::meter(
        Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        },
        buf,
        ratio,
        label,
        &format!(
            "{} / {} ({:.1}%)",
            util::human_bytes(used.unwrap_or(0)),
            util::human_bytes(total.unwrap_or(0)),
            ratio.unwrap_or(0.0) * 100.0
        ),
        theme,
        kind,
    );
    let graph = Rect {
        x: area.x,
        y: area.y + 1,
        width: area.width,
        height: area.height.saturating_sub(1),
    };
    let values = series(hist);
    if graph.width > 2 && graph.height > 1 && !values.is_empty() {
        widgets::graph(
            graph,
            buf,
            &values,
            total.unwrap_or(1) as f64,
            theme,
            kind,
        );
    }
}

fn draw_process_totals(area: Rect, buf: &mut Buffer, snap: &Snapshot, theme: &Theme) {
    if area.width < 12 || area.height < 3 {
        return;
    }
    let mut gtt = 0u64;
    let mut vram = 0u64;
    let mut sys = 0u64;
    let mut evicted = 0u64;
    let mut clients = 0usize;
    let mut top: Vec<_> = snap
        .procs
        .iter()
        .filter(|proc| proc.gpu_clients > 0)
        .collect();
    for proc in &snap.procs {
        gtt += proc.gtt;
        vram += proc.vram;
        sys += proc.gpu_sys;
        evicted += proc.evicted_vram;
        clients += proc.gpu_clients;
    }
    top.sort_by(|a, b| b.gtt.cmp(&a.gtt));
    let mut lines = vec![
        widgets::kv_styled(
            "GTT",
            format!("{} across {} clients", util::human_bytes(gtt), clients),
            Style::default().fg(crate::theme::GTT_ACCENT),
            theme,
        ),
        widgets::kv_styled(
            "VRAM",
            util::human_bytes(vram),
            Style::default().fg(crate::theme::VRAM_ACCENT),
            theme,
        ),
        widgets::kv("sysmem", util::human_bytes(sys), theme),
        widgets::kv("evicted", util::human_bytes(evicted), theme),
    ];
    if area.height as usize > lines.len() + 1 {
        lines.push(Line::from(Span::styled(
            "top GTT holders",
            Style::default().fg(crate::theme::DIM),
        )));
        for proc in top.iter().take(area.height.saturating_sub(lines.len() as u16 + 1) as usize) {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{:>7} ", proc.pid),
                    Style::default().fg(crate::theme::DIM),
                ),
                Span::styled(
                    util::human_bytes(proc.gtt),
                    Style::default().fg(crate::theme::GTT_ACCENT),
                ),
                Span::styled(
                    format!(" {}", util::truncate(&proc.comm, 18)),
                    Style::default().fg(crate::theme::STRONG),
                ),
            ]));
        }
    }
    widgets::lines(area, buf, lines);
}

/// TTM view: debugfs resource managers, page pool and the module limits.
fn view_ttm(area: Rect, buf: &mut Buffer, snap: &Snapshot, theme: &Theme) {
    let ttm = &snap.ttm;
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);
    let left = widgets::panel(theme.block("ttm resource managers (debugfs)", crate::theme::MEM_BOX), columns[0], buf);
    if !ttm.available {
        let mut lines = vec![
            Line::from(Span::styled(
                "TTM internals are not readable",
                Style::default()
                    .fg(crate::theme::WARN)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                ttm.error.clone().unwrap_or_default(),
                Style::default().fg(crate::theme::TEXT),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "sudo amdtop          # full TTM view",
                Style::default().fg(crate::theme::ACCENT),
            )),
            Line::from(Span::styled(
                "VRAM/GTT totals above do not need root.",
                Style::default().fg(crate::theme::DIM),
            )),
        ];
        if let Some(path) = &ttm.path {
            lines.push(widgets::kv("path", path.clone(), theme));
        }
        widgets::lines(left, buf, lines);
    } else {
        let mut lines = Vec::new();
        if let Some(path) = &ttm.path {
            lines.push(widgets::kv("path", util::truncate(path, 40), theme));
        }
        for (label, stats, color) in [
            ("vram_mm", &ttm.vram, crate::theme::VRAM_ACCENT),
            ("gtt_mm", &ttm.gtt, crate::theme::GTT_ACCENT),
        ] {
            match stats {
                Some(stats) => {
                    lines.push(Line::from(Span::styled(
                        label,
                        Style::default()
                            .fg(color)
                            .add_modifier(Modifier::BOLD),
                    )));
                    lines.push(widgets::kv(
                        "size",
                        format!("{} ({} pages)", util::human_bytes(stats.size.unwrap_or(0)), stats.size.unwrap_or(0) / ttm.page_size.max(1)),
                        theme,
                    ));
                    lines.push(widgets::kv_styled(
                        "usage",
                        format!(
                            "{} ({:.1}%)",
                            util::human_bytes(stats.usage.unwrap_or(0)),
                            stats.ratio().unwrap_or(0.0) * 100.0
                        ),
                        Style::default().fg(color),
                        theme,
                    ));
                    let flag = |value: Option<i64>| {
                        value.map(|v| v.to_string()).unwrap_or_else(|| "-".to_string())
                    };
                    lines.push(widgets::kv(
                        "flags",
                        format!("use_type {}  use_tt {}", flag(stats.use_type), flag(stats.use_tt)),
                        theme,
                    ));
                }
                None => lines.push(widgets::kv(label, "not exported".to_string(), theme)),
            }
        }
        widgets::lines(left, buf, lines);
    }

    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(columns[1]);
    let pool_inner = widgets::panel(theme.block("ttm_page_pool (cached page orders)", crate::theme::MEM_BOX), right[0], buf);
    let mut lines = Vec::new();
    if ttm.pool.unused {
        lines.push(Line::from(Span::styled("pool unused", Style::default().fg(crate::theme::DIM))));
    }
    for (label, counts) in ttm.pool.rows.iter().take(6) {
        let text: Vec<String> = counts.iter().take(5).map(|value| format!("{:>7}", value)).collect();
        lines.push(Line::from(vec![
            Span::styled(format!("{:<8}", label), Style::default().fg(crate::theme::DIM)),
            Span::styled(text.join(""), Style::default().fg(crate::theme::STRONG)),
        ]));
    }
    for (node, used, limit) in &ttm.pool.totals {
        let ratio = if *limit > 0 {
            *used as f64 / *limit as f64
        } else {
            0.0
        };
        lines.push(widgets::kv(
            node,
            format!(
                "{} / {} ({:.1}%)",
                util::human_bytes(*used * ttm.page_size),
                util::human_bytes(*limit * ttm.page_size),
                ratio * 100.0
            ),
            theme,
        ));
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "no page pool data (needs root)",
            Style::default().fg(crate::theme::DIM),
        )));
    }
    widgets::lines(pool_inner, buf, lines);

    let params_inner = widgets::panel(theme.block("module parameters", crate::theme::MEM_BOX), right[1], buf);
    let mut lines: Vec<Line> = ttm
        .params
        .iter()
        .map(|(name, value)| widgets::kv(name, value.clone(), theme))
        .collect();
    if let Some(gtt) = snap.gpu.as_ref().and_then(|g| g.gtt_total) {
        lines.push(widgets::kv_styled(
            "gtt limit",
            util::human_bytes(gtt),
            Style::default().fg(crate::theme::GTT_ACCENT).add_modifier(Modifier::BOLD),
            theme,
        ));
        lines.push(Line::from(Span::styled(
            "  = ttm.pages_limit × page size",
            Style::default().fg(crate::theme::DIM),
        )));
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "no parameters readable",
            Style::default().fg(crate::theme::DIM),
        )));
    }
    widgets::lines(params_inner, buf, lines);
}

/// Unified view: one physical pool split between CPU and GPU.
fn view_unified(area: Rect, buf: &mut Buffer, snap: &Snapshot, hist: &History, theme: &Theme) {
    let mem = &snap.mem;
    let gtt = snap.gpu.as_ref().and_then(|g| g.gtt_used).unwrap_or(0);
    let vram_total = snap.gpu.as_ref().and_then(|g| g.vram_total).unwrap_or(0);
    let cpu_only = mem.used.saturating_sub(gtt);
    let free = mem.total.saturating_sub(mem.used);
    let segments = vec![
        (cpu_only as f64, crate::theme::CPU_ACCENT),
        (gtt as f64, crate::theme::GTT_ACCENT),
        (free as f64, crate::theme::DIM),
    ];
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Length(6),
            Constraint::Min(4),
        ])
        .split(area);

    let physical = widgets::panel(
        theme.block(&format!("physical pool seen by the kernel · {}", util::human_bytes(mem.total)), crate::theme::MEM_BOX),
        rows[0],
        buf,
    );
    widgets::stacked(
        Rect {
            x: physical.x,
            y: physical.y,
            width: physical.width,
            height: 1,
        },
        buf,
        &segments,
        theme,
    );
    widgets::lines(
        Rect {
            x: physical.x,
            y: physical.y + 1,
            width: physical.width,
            height: physical.height.saturating_sub(1),
        },
        buf,
        vec![widgets::legend(
            &[
                (crate::theme::CPU_ACCENT, format!("CPU {}", util::human_bytes(cpu_only))),
                (crate::theme::GTT_ACCENT, format!("GTT {}", util::human_bytes(gtt))),
                (crate::theme::DIM, format!("free {}", util::human_bytes(free))),
            ],
            theme,
        )],
    );

    let carveout = widgets::panel(theme.block("outside the kernel pool", crate::theme::MEM_BOX), rows[1], buf);
    widgets::meter(
        Rect {
            x: carveout.x,
            y: carveout.y,
            width: carveout.width,
            height: 1,
        },
        buf,
        util::ratio(
            snap.gpu.as_ref().and_then(|g| g.vram_used),
            Some(vram_total.max(1)),
        ),
        "VRAM",
        &format!(
            "{} / {} carved out by the BIOS",
            util::human_bytes(snap.gpu.as_ref().and_then(|g| g.vram_used).unwrap_or(0)),
            util::human_bytes(vram_total)
        ),
        theme,
        Gradient::Available,
    );

    let graph_inner = widgets::panel(theme.block("CPU vs GTT over time", crate::theme::MEM_BOX), rows[2], buf);
    if graph_inner.width > 2 && graph_inner.height > 1 {
        widgets::graph(
            graph_inner,
            buf,
            &hist.gtt.values(),
            mem.total as f64,
            theme,
            Gradient::Download,
        );
    }

    let details = widgets::panel(theme.block("breakdown", crate::theme::MEM_BOX), rows[3], buf);
    let total = mem.total.max(1) as f64;
    let lines = vec![
        Line::from(vec![
            Span::styled("segment   ", Style::default().fg(crate::theme::DIM)),
            Span::styled("bytes", Style::default().fg(crate::theme::DIM)),
            Span::styled("     share", Style::default().fg(crate::theme::DIM)),
        ]),
        breakdown_line("CPU", cpu_only, total, crate::theme::CPU_ACCENT, theme),
        breakdown_line("GTT", gtt, total, crate::theme::GTT_ACCENT, theme),
        breakdown_line("cache", mem.cached.min(mem.used), total, crate::theme::WARN, theme),
        breakdown_line("free", free, total, crate::theme::DIM, theme),
        breakdown_line("VRAM", vram_total, total, crate::theme::VRAM_ACCENT, theme),
        Line::from(Span::styled(
            format!(
                "GPU total (VRAM+GTT) {} · {:.1}% of the pool",
                util::human_bytes(gtt + vram_total),
                (gtt + vram_total) as f64 / total * 100.0
            ),
            Style::default()
                .fg(crate::theme::GPU_ACCENT)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "shares are of MemTotal; GTT and cache are subsets of `used`",
            Style::default().fg(crate::theme::DIM),
        )),
    ];
    widgets::lines(details, buf, lines);
    let _ = hist;
}

fn breakdown_line(label: &str, value: u64, total: f64, color: ratatui::style::Color, _theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{:<10}", label), Style::default().fg(crate::theme::DIM)),
        Span::styled(
            util::human_bytes(value),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{:>10.1}%", value as f64 / total * 100.0),
            Style::default().fg(crate::theme::TEXT),
        ),
    ])
}

