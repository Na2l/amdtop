//! GPU panels: busy graph, VRAM/GTT gauges, clocks, power, temperatures.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::collect::{GpuSample, History, Snapshot};
use crate::theme::{Gradient, Theme};
use crate::util;

use super::widgets;
use super::App;

/// btop-style GPU box: usage histogram on top, then GPU / PWR / VRAM / GTT bars.
///
/// This mirrors btop's `gpu0` box, with GTT added because on Strix Halo the
/// pageable system-RAM pool matters as much as the VRAM carveout.
pub fn btop_box(area: Rect, buf: &mut Buffer, snap: &Snapshot, _hist: &History, theme: &Theme) {
    let title = match &snap.gpu {
        Some(gpu) => format!("gpu0 · {}", util::truncate(&gpu.marketing, 30)),
        None => "gpu0".to_string(),
    };
    let inner = widgets::panel(theme.block(&title, crate::theme::GPU_BOX), area, buf);
    let Some(gpu) = &snap.gpu else {
        widgets::lines(
            inner,
            buf,
            vec![
                Line::from(Span::styled(
                    "no amdgpu device found",
                    Style::default().fg(crate::theme::WARN),
                )),
                Line::from(Span::styled(
                    "run `amdtop --demo` to see the layout",
                    Style::default().fg(theme.dim()),
                )),
            ],
        );
        return;
    };
    if inner.width < 14 || inner.height < 3 {
        return;
    }

    // Data rows: GPU, PWR, VRAM and GTT (the last is the Strix Halo one).
    let rows = 4.min(inner.height) as usize;
    let mut y = inner.y;
    let busy = gpu.busy.unwrap_or(0.0);
    widgets::meter(
        Rect { x: inner.x, y, width: inner.width, height: 1 },
        buf,
        Some(busy),
        "GPU",
        &format!("{:>5.1}% {}", busy * 100.0, util::human_freq(gpu.sclk)),
        theme,
        Gradient::Cpu,
    );
    y += 1;
    if rows >= 2 {
        let cap = gpu.power_cap.unwrap_or(0.0);
        widgets::meter(
            Rect { x: inner.x, y, width: inner.width, height: 1 },
            buf,
            gpu.power_w
                .map(|power| if cap > 0.0 { power / cap } else { power / 120.0 }),
            "PWR",
            &match gpu.power_w {
                Some(power) => format!("{:>5.1}W", power),
                None => "    -".to_string(),
            },
            theme,
            Gradient::Temp,
        );
        y += 1;
    }
    if rows >= 3 {
        widgets::meter(
            Rect { x: inner.x, y, width: inner.width, height: 1 },
            buf,
            gpu.vram_ratio(),
            "VRAM",
            &format!(
                "{} / {}",
                util::human_bytes(gpu.vram_used.unwrap_or(0)),
                util::human_bytes(gpu.vram_total.unwrap_or(0))
            ),
            theme,
            Gradient::Available,
        );
        y += 1;
    }
    if rows >= 4 {
        widgets::meter(
            Rect { x: inner.x, y, width: inner.width, height: 1 },
            buf,
            gpu.gtt_ratio(),
            "GTT",
            &format!(
                "{} / {}",
                util::human_bytes(gpu.gtt_used.unwrap_or(0)),
                util::human_bytes(gpu.gtt_total.unwrap_or(0))
            ),
            theme,
            Gradient::Download,
        );
        y += 1;
    }
    // Any remaining rows become a sensor summary.
    if (y as usize) < inner.bottom() as usize {
        let mut lines = vec![Line::from(vec![
            Span::styled(
                gpu.hottest()
                    .map(|(label, value)| format!("{} {:.0}°C", label, value))
                    .unwrap_or_else(|| "temp -".to_string()),
                Style::default()
                    .fg(theme.grad(Gradient::Temp, gpu.hottest().map(|(_, v)| v / 100.0).unwrap_or(0.0))),
            ),
            Span::styled(
                format!(
                    "   vram {}   gtt {}",
                    util::human_freq(gpu.mclk),
                    util::human_freq(gpu.sclk)
                ),
                Style::default().fg(crate::theme::GRAPH_TEXT),
            ),
        ])];
        if let Some(cap) = gpu.power_cap {
            lines.push(widgets::kv(
                "power cap",
                format!("{:.0} W", cap),
                theme,
            ));
        }
        widgets::lines(
            Rect {
                x: inner.x,
                y,
                width: inner.width,
                height: inner.height.saturating_sub(y - inner.y),
            },
            buf,
            lines,
        );
    }
}

/// GPU usage histogram, drawn to the left of the data box.
pub fn histogram_box(
    area: Rect,
    buf: &mut Buffer,
    snap: &Snapshot,
    hist: &History,
    theme: &Theme,
    mirrored: bool,
) {
    let title = match &snap.gpu {
        Some(gpu) => format!("gpu0 usage · {}", util::truncate(&gpu.marketing, 34)),
        None => "gpu0 usage".to_string(),
    };
    let inner = widgets::panel(theme.block(&title, crate::theme::GPU_BOX), area, buf);
    if inner.width < 8 || inner.height < 2 {
        return;
    }
    widgets::usage_graph(inner, buf, &hist.gpu.values(), 1.0, theme, Gradient::Cpu, mirrored);
}

/// Full GPU tab.
pub fn full(area: Rect, buf: &mut Buffer, _app: &App, snap: &Snapshot, hist: &History, theme: &Theme) {
    let Some(gpu) = &snap.gpu else {
        let inner = widgets::panel(theme.block("GPU", crate::theme::GPU_BOX), area, buf);
        widgets::lines(
            inner,
            buf,
            vec![Line::from(Span::styled(
                "no amdgpu device found — try `amdtop --demo`",
                Style::default().fg(crate::theme::WARN),
            ))],
        );
        return;
    };
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(area);

    // Left: busy graph + engine utilisation.
    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(columns[0]);
    let graph_inner = widgets::panel(
        theme.block(
            &format!(
                "GPU busy {} · {}",
                util::human_pct(gpu.busy),
                util::truncate(&gpu.marketing, 24)
            ),
            crate::theme::GPU_BOX,
        ),
        left[0],
        buf,
    );
    if graph_inner.width > 2 && graph_inner.height > 1 {
        widgets::graph(
            graph_inner,
            buf,
            &hist.gpu.values(),
            1.0,
            theme,
            Gradient::Cpu,
        );
    }
    let engines_inner = widgets::panel(theme.block("engines (per-client fdinfo)", crate::theme::GPU_BOX), left[1], buf);
    draw_engines(engines_inner, buf, snap, theme);

    // Right: memory gauges above the sensor readouts.
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(8), Constraint::Min(4)])
        .split(columns[1]);
    let mem_inner = widgets::panel(theme.block("GPU memory", crate::theme::GPU_BOX), right[0], buf);
    draw_memory_gauges(mem_inner, buf, gpu, theme);
    let stats_inner = widgets::panel(theme.block("sensors", crate::theme::GPU_BOX), right[1], buf);
    draw_sensors(stats_inner, buf, gpu, hist, theme);
}

fn draw_engines(area: Rect, buf: &mut Buffer, snap: &Snapshot, theme: &Theme) {
    if area.width < 12 || area.height == 0 {
        return;
    }
    if snap.engines.is_empty() {
        widgets::lines(
            area,
            buf,
            vec![Line::from(Span::styled(
                "no GPU clients seen (no /dev/dri fds readable)",
                Style::default().fg(crate::theme::DIM),
            ))],
        );
        return;
    }
    let right_width = 8;
    for (row, (engine, ratio)) in snap.engines.iter().enumerate().take(area.height as usize) {
        let y = area.top() + row as u16;
        let label = util::fit(&util::truncate(engine, 12), 13);
        buf.set_line(
            area.left(),
            y,
            &Line::from(Span::styled(label, Style::default().fg(crate::theme::DIM))),
            13,
        );
        let bar_width = area.width.saturating_sub(13 + right_width + 1) as usize;
        widgets::meter(
            Rect {
                x: area.left() + 13,
                y,
                width: bar_width as u16,
                height: 1,
            },
            buf,
            Some(*ratio),
            "",
            "",
            theme,
            Gradient::Cpu,
        );
        buf.set_line(
            area.right().saturating_sub(right_width),
            y,
            &Line::from(Span::styled(
                format!("{:>6.1}%", ratio * 100.0),
                Style::default().fg(crate::theme::STRONG),
            )),
            right_width,
        );
    }
}

fn draw_memory_gauges(area: Rect, buf: &mut Buffer, gpu: &GpuSample, theme: &Theme) {
    if area.width < 12 || area.height < 3 {
        return;
    }
    let rows = [
        ("VRAM", gpu.vram_used, gpu.vram_total, Gradient::Available, "carveout"),
        ("GTT", gpu.gtt_used, gpu.gtt_total, Gradient::Download, "system RAM"),
        ("vis", gpu.vis_vram_used, gpu.vis_vram_total, Gradient::Cached, "BAR window"),
    ];
    for (row, (label, used, total, kind, note)) in rows.iter().enumerate().take(area.height as usize) {
        let y = area.top() + row as u16 * 2;
        if y >= area.bottom() {
            break;
        }
        let right = format!(
            "{} / {}",
            util::human_bytes_compact(used.unwrap_or(0)),
            util::human_bytes_compact(total.unwrap_or(0))
        );
        widgets::meter(
            Rect {
                x: area.left(),
                y,
                width: area.width,
                height: 1,
            },
            buf,
            util::ratio(*used, *total),
            label,
            &right,
            theme,
            *kind,
        );
        let note_line = Rect {
            x: area.left(),
            y: y + 1,
            width: area.width,
            height: 1,
        };
        if note_line.y < area.bottom() {
            buf.set_line(
                note_line.x,
                note_line.y,
                &Line::from(Span::styled(
                    format!("  {} · {}", note, util::human_pct(util::ratio(*used, *total))),
                    Style::default().fg(crate::theme::DIM),
                )),
                area.width,
            );
        }
    }
}

fn draw_sensors(area: Rect, buf: &mut Buffer, gpu: &GpuSample, hist: &History, theme: &Theme) {
    if area.width < 10 || area.height < 3 {
        return;
    }
    let mut lines = Vec::new();
    lines.push(widgets::kv(
        "clock",
        format!(
            "{} ({} … {})",
            util::human_freq(gpu.sclk),
            util::human_freq(gpu.sclk_min),
            util::human_freq(gpu.sclk_max)
        ),
        theme,
    ));
    lines.push(widgets::kv(
        "vram clk",
        format!(
            "{} ({} … {})",
            util::human_freq(gpu.mclk),
            util::human_freq(gpu.mclk_min),
            util::human_freq(gpu.mclk_max)
        ),
        theme,
    ));
    for (label, temp) in &gpu.temps {
        lines.push(widgets::kv_styled(
            label,
            format!("{:.0}°C", temp),
            Style::default().fg(temperature_color(Some(*temp))),
            theme,
        ));
    }
    if let Some(power) = gpu.power_w {
        lines.push(widgets::kv_styled(
            "power",
            format!(
                "{:.1} W{}",
                power,
                gpu.power_cap
                    .map(|cap| format!(" / {:.0} W", cap))
                    .unwrap_or_default()
            ),
            Style::default().fg(crate::theme::STRONG),
            theme,
        ));
    }
    if let Some(rpm) = gpu.fan_rpm {
        lines.push(widgets::kv("fan", format!("{:.0} RPM", rpm), theme));
    }
    lines.push(widgets::kv(
        "mem busy",
        util::human_pct(gpu.mem_busy),
        theme,
    ));
    lines.push(widgets::kv("driver", format!("{} · {}", gpu.driver, gpu.pci_slot), theme));
    if !hist.gpu_power.is_empty() {
        let peak = hist.gpu_power.max().unwrap_or(1.0).max(1.0);
        lines.push(widgets::spark(
            &hist.gpu_power.values(),
            area.width as usize,
            peak,
            theme,
            Gradient::Cached,
        ));
    }
    widgets::lines(area, buf, lines);
}


fn temperature_color(temp: Option<f64>) -> ratatui::style::Color {
    match temp {
        Some(value) if value >= 95.0 => crate::theme::BAD,
        Some(value) if value >= 80.0 => crate::theme::WARN,
        Some(_) => crate::theme::GOOD,
        None => crate::theme::DIM,
    }
}
