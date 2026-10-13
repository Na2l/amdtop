//! CPU panels: btop's histogram-style CPU box plus a detailed CPU tab.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::collect::{History, Snapshot};
use crate::theme::{self, Gradient, Theme};
use crate::util;

use super::widgets;
use super::App;

/// Rows the layout should reserve above the core grid: the total meter plus
/// uptime, load average and process counts, as btop shows them.
pub fn head_rows(area_height: u16) -> u16 {
    if area_height >= 24 {
        4
    } else {
        2
    }
}

/// btop's CPU box: total meter plus histogram on the top row (the histogram
/// starts at the middle of the panel), then every core as a compact cell on the
/// full width below.
pub fn btop_box(
    area: Rect,
    buf: &mut Buffer,
    app: &App,
    snap: &Snapshot,
    hist: &History,
    theme: &Theme,
) {
    let title = format!(
        "cpu · {} threads{}",
        snap.cpu.nproc,
        if snap.cpu.siblings.is_empty() {
            String::new()
        } else {
            format!(" · {} cores", snap.cpu.siblings.len())
        }
    );
    let inner = widgets::panel(theme.block(&title, theme::CPU_BOX), area, buf);
    if inner.width < 20 || inner.height == 0 {
        return;
    }

    // Head rows: total meter, uptime, load average and process count. The grid is
    // sized first so the head takes exactly the rows that are left over.
    let cores = snap.cpu.per_core.len();
    let cell_min: u16 = 18;
    let columns = ((inner.width / cell_min).max(1) as usize).min(cores.max(1));
    let core_rows_needed = cores.div_ceil(columns);
    let head_height = (inner.height as usize)
        .saturating_sub(core_rows_needed)
        .clamp(1, 4) as u16;
    widgets::meter(
        Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: 1,
        },
        buf,
        Some(snap.cpu.total),
        "CPU",
        &format!("{:>5.1}%", snap.cpu.total * 100.0),
        theme,
        Gradient::Cpu,
    );
    if head_height >= 2 {
        let mut lines = vec![Line::from(Span::styled(
            format!("up {}", util::human_seconds(snap.cpu.uptime)),
            Style::default().fg(theme.dim()),
        ))];
        if head_height >= 3 {
            lines.push(Line::from(Span::styled(
                format!(
                    "load {:.2} {:.2} {:.2}",
                    snap.cpu.load[0], snap.cpu.load[1], snap.cpu.load[2]
                ),
                Style::default().fg(crate::theme::GRAPH_TEXT),
            )));
        }
        if head_height >= 4 {
            lines.push(Line::from(Span::styled(
                format!(
                    "{} running / {} procs",
                    snap.cpu.procs_running.unwrap_or(0),
                    snap.cpu.procs_total.unwrap_or(0)
                ),
                Style::default().fg(theme.dim()),
            )));
        }
        widgets::lines(
            Rect {
                x: inner.x,
                y: inner.y + 1,
                width: inner.width,
                height: head_height - 1,
            },
            buf,
            lines,
        );
    }

    // Cores: full width, btop-sized compact cells.
    let grid = Rect {
        x: inner.x,
        y: inner.y + head_height,
        width: inner.width,
        height: inner.height.saturating_sub(head_height),
    };
    let available = grid.height as usize;
    let max_scroll = core_rows_needed.saturating_sub(available.max(1));
    let scroll = app.cpu_scroll.min(max_scroll);
    let cell_width = grid.width / columns as u16;
    let label_width = 4u16;
    let right_width = if cell_width >= 30 { 14u16 } else { 6u16 };
    let hist_cell = cell_width
        .saturating_sub(label_width + right_width + 1)
        .max(3);
    for row in 0..available {
        let first = (row + scroll) * columns;
        if first >= cores {
            break;
        }
        for column in 0..columns {
            let core = first + column;
            if core >= cores {
                break;
            }
            let x0 = grid.x + column as u16 * cell_width;
            let y = grid.y + row as u16;
            let load = snap.cpu.per_core[core];
            for (offset, ch) in format!("C{:02} ", core).chars().enumerate() {
                if let Some(cell) = buf.cell_mut((x0 + offset as u16, y)) {
                    cell.set_symbol(&ch.to_string());
                    cell.set_style(Style::default().fg(theme.label()));
                }
            }
            let values = hist
                .per_core
                .get(core)
                .map(|ring| ring.values())
                .unwrap_or_default();
            let line = widgets::spark(&values, hist_cell as usize, 1.0, theme, Gradient::Cpu);
            buf.set_line(x0 + label_width, y, &line, hist_cell);
            let freq = snap
                .cpu
                .freq_mhz
                .get(core)
                .and_then(|value| *value)
                .map(|mhz| format!("{:.2}G", mhz / 1000.0))
                .unwrap_or_else(|| "-".to_string());
            let value = if right_width > 8 {
                format!("{:>5.1}% {:>7}", load * 100.0, freq)
            } else {
                format!("{:>5.1}%", load * 100.0)
            };
            let vx = x0 + cell_width.saturating_sub(right_width + 1);
            for (offset, ch) in value.chars().enumerate() {
                if let Some(cell) = buf.cell_mut((vx + offset as u16, y)) {
                    cell.set_symbol(&ch.to_string());
                    cell.set_style(widgets::load_style(load, theme, Gradient::Cpu));
                }
            }
        }
    }
}

/// CPU usage histogram in its own panel, mirroring the GPU row's histogram box.
pub fn histogram_box(
    area: Rect,
    buf: &mut Buffer,
    snap: &Snapshot,
    hist: &History,
    theme: &Theme,
    mirrored: bool,
) {
    let title = format!(
        "cpu total · load {:.2} {:.2} {:.2}",
        snap.cpu.load[0], snap.cpu.load[1], snap.cpu.load[2]
    );
    let inner = widgets::panel(theme.block(&title, theme::CPU_BOX), area, buf);
    if inner.width < 6 || inner.height < 2 {
        return;
    }
    widgets::usage_graph(inner, buf, &hist.cpu.values(), 1.0, theme, Gradient::Cpu, mirrored);
}

/// CPU detail tab: big graph, every core, frequencies and temperatures.
pub fn full(area: Rect, buf: &mut Buffer, app: &App, snap: &Snapshot, hist: &History, theme: &Theme) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
        .split(area);

    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(9), Constraint::Min(3)])
        .split(columns[0]);
    let graph_inner = widgets::panel(theme.block("CPU total", theme::CPU_BOX), left[0], buf);
    if graph_inner.width > 2 && graph_inner.height > 1 {
        widgets::graph(graph_inner, buf, &hist.cpu.values(), 1.0, theme, Gradient::Cpu);
    }

    let cores_inner = widgets::panel(
        theme.block(
            &format!(
                "cores {} · {}",
                snap.cpu.per_core.len(),
                if app.cpu_scroll > 0 {
                    format!("offset {}", app.cpu_scroll)
                } else {
                    "up/down to scroll".to_string()
                }
            ),
            theme::CPU_BOX,
        ),
        left[1],
        buf,
    );
    draw_core_rows(cores_inner, buf, snap, app, hist, theme);

    let stats_inner = widgets::panel(theme.block("stats", theme::CPU_BOX), columns[1], buf);
    draw_stats(stats_inner, buf, snap, hist, theme);
}

fn draw_core_rows(
    area: Rect,
    buf: &mut Buffer,
    snap: &Snapshot,
    app: &App,
    hist: &History,
    theme: &Theme,
) {
    if area.width < 20 || area.height == 0 {
        return;
    }
    let label_width = 5u16;
    let right_width = 15u16;
    let hist_width = area
        .width
        .saturating_sub(label_width + right_width + 2)
        .max(4);
    let visible = area.height as usize;
    let scroll = app.cpu_scroll.min(snap.cpu.per_core.len().saturating_sub(1));
    for (row, core) in (scroll..snap.cpu.per_core.len()).enumerate().take(visible) {
        let load = snap.cpu.per_core[core];
        let y = area.top() + row as u16;
        for (offset, ch) in format!("C{:02} ", core).chars().enumerate() {
            if let Some(cell) = buf.cell_mut((area.left() + offset as u16, y)) {
                cell.set_symbol(&ch.to_string());
                cell.set_style(Style::default().fg(theme.label()));
            }
        }
        let values = hist
            .per_core
            .get(core)
            .map(|ring| ring.values())
            .unwrap_or_default();
        let line = widgets::spark(&values, hist_width as usize, 1.0, theme, Gradient::Cpu);
        buf.set_line(area.left() + label_width, y, &line, hist_width);
        let freq = snap
            .cpu
            .freq_mhz
            .get(core)
            .and_then(|value| *value)
            .map(|mhz| format!("{:.2}G", mhz / 1000.0))
            .unwrap_or_else(|| "-".to_string());
        let value = format!("{:>5.1}% {:>7}", load * 100.0, freq);
        let vx = area.right().saturating_sub(right_width + 1);
        for (offset, ch) in value.chars().enumerate() {
            if let Some(cell) = buf.cell_mut((vx + offset as u16, y)) {
                cell.set_symbol(&ch.to_string());
                cell.set_style(widgets::load_style(load, theme, Gradient::Cpu));
            }
        }
    }
}

fn draw_stats(area: Rect, buf: &mut Buffer, snap: &Snapshot, hist: &History, theme: &Theme) {
    if area.width < 14 || area.height < 4 {
        return;
    }
    let mut lines: Vec<Line<'static>> = Vec::new();
    let freq_values: Vec<f64> = snap.cpu.freq_mhz.iter().flatten().copied().collect();
    let min_freq = freq_values
        .iter()
        .copied()
        .fold(None, |acc: Option<f64>, v| Some(acc.map_or(v, |a| a.min(v))));
    let max_freq = snap.cpu.max_freq_mhz();
    lines.push(widgets::kv(
        "model",
        util::truncate(&snap.cpu.model, (area.width as usize).saturating_sub(12).max(4)),
        theme,
    ));
    lines.push(widgets::kv("threads", snap.cpu.nproc.to_string(), theme));
    if !snap.cpu.siblings.is_empty() {
        lines.push(widgets::kv(
            "cores",
            format!(
                "{} physical (SMT {})",
                snap.cpu.siblings.len(),
                snap.cpu.nproc as f64 / snap.cpu.siblings.len() as f64
            ),
            theme,
        ));
    }
    lines.push(widgets::kv(
        "freq",
        format!(
            "{} … {}",
            util::human_freq(min_freq),
            util::human_freq(max_freq)
        ),
        theme,
    ));
    lines.push(widgets::kv(
        "load",
        format!(
            "{:.2} {:.2} {:.2}",
            snap.cpu.load[0], snap.cpu.load[1], snap.cpu.load[2]
        ),
        theme,
    ));
    lines.push(widgets::kv(
        "procs",
        format!(
            "{} running / {}",
            snap.cpu.procs_running.unwrap_or(0),
            snap.cpu.procs_total.unwrap_or(0)
        ),
        theme,
    ));
    lines.push(widgets::kv("uptime", util::human_seconds(snap.cpu.uptime), theme));
    if let Some(temp) = snap.cpu.temp {
        let label = if snap.cpu.temp_label.is_empty() {
            "temp".to_string()
        } else {
            snap.cpu.temp_label.clone()
        };
        let ratio = snap.cpu.temp_max.map(|max| temp / max).unwrap_or(0.5);
        lines.push(widgets::kv_styled(
            &label,
            format!(
                "{:.0}°C{}",
                temp,
                snap.cpu
                    .temp_max
                    .map(|m| format!(" / {:.0}°C", m))
                    .unwrap_or_default()
            ),
            Style::default().fg(theme.grad(Gradient::Temp, ratio)),
            theme,
        ));
    }
    if snap.cpu.steal > 0.001 {
        lines.push(widgets::kv(
            "steal",
            format!("{:.1}%", snap.cpu.steal * 100.0),
            theme,
        ));
    }
    if area.height as usize > lines.len() + 3 {
        lines.push(Line::from(Span::styled(
            "history",
            Style::default().fg(theme.dim()),
        )));
        let width = area.width as usize;
        lines.push(widgets::spark(&hist.cpu.values(), width, 1.0, theme, Gradient::Cpu));
        lines.push(widgets::spark(
            &hist.cpu_temp.values(),
            width,
            snap.cpu.temp_max.unwrap_or(100.0),
            theme,
            Gradient::Temp,
        ));
    }
    widgets::lines(area, buf, lines);
}
