//! NPU panels: usage histogram, per-column load and hardware contexts.
//!
//! The data comes from the amdxdna query ioctl, not sysfs — see
//! `collect/npu.rs`. Power and per-column utilization need the kernel's AMD
//! PMF NPU metrics; when that is unavailable the panel still shows clocks,
//! TOPS, the array shape and live contexts.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::collect::{History, NpuContext, NpuSample, Snapshot};
use crate::theme::{Gradient, Theme};
use crate::util;

use super::widgets;
use super::App;

/// Full NPU tab.
pub fn full(
    area: Rect,
    buf: &mut Buffer,
    _app: &App,
    snap: &Snapshot,
    hist: &History,
    theme: &Theme,
) {
    let Some(npu) = &snap.npu else {
        let inner = widgets::panel(theme.block("NPU", crate::theme::NPU_BOX), area, buf);
        widgets::lines(
            inner,
            buf,
            vec![
                Line::from(Span::styled(
                    "no amdxdna device found",
                    Style::default().fg(crate::theme::WARN),
                )),
                Line::from(Span::styled(
                    "looking for /sys/class/accel/accel* with driver=amdxdna",
                    Style::default().fg(theme.dim()),
                )),
                Line::from(Span::styled(
                    "run `amdtop --demo` to see the layout",
                    Style::default().fg(theme.dim()),
                )),
            ],
        );
        return;
    };
    if area.width < 40 || area.height < 12 {
        return;
    }

    let table_height = (npu.contexts.len() as u16 + 3).clamp(4, 12);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(table_height)])
        .split(area);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(rows[0]);

    graph_box(columns[0], buf, npu, hist, theme);
    stats_box(columns[1], buf, npu, hist, theme);
    contexts_box(rows[1], buf, snap, npu, theme);
}

/// Compact NPU row for the dashboard, directly under the GPU row.
pub fn dashboard(
    area: Rect,
    buf: &mut Buffer,
    snap: &Snapshot,
    hist: &History,
    theme: &Theme,
    mirrored: bool,
) {
    let Some(npu) = &snap.npu else {
        return;
    };
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(area);
    dashboard_graph(columns[0], buf, npu, hist, theme, mirrored);
    dashboard_stats(columns[1], buf, npu, theme);
}

fn dashboard_graph(
    area: Rect,
    buf: &mut Buffer,
    npu: &NpuSample,
    hist: &History,
    theme: &Theme,
    mirrored: bool,
) {
    let title = match npu.busy {
        Some(busy) => format!(
            "npu usage {} · {}",
            util::human_pct(Some(busy)).trim(),
            util::truncate(npu.label(), 26)
        ),
        None => format!(
            "npu active cols {} · {}",
            util::human_pct(npu.cols_ratio()).trim(),
            util::truncate(npu.label(), 22)
        ),
    };
    let inner = widgets::panel(theme.block(&title, crate::theme::NPU_BOX), area, buf);
    if inner.width < 8 || inner.height < 2 {
        return;
    }
    let graph_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: inner.height.saturating_sub(1),
    };
    widgets::usage_graph(
        graph_area,
        buf,
        &hist.npu.values(),
        1.0,
        theme,
        Gradient::Cpu,
        mirrored,
    );
    let strip = Rect {
        x: inner.x,
        y: inner.bottom().saturating_sub(1),
        width: inner.width,
        height: 1,
    };
    column_strip(strip, buf, npu, theme);
}

fn dashboard_stats(area: Rect, buf: &mut Buffer, npu: &NpuSample, theme: &Theme) {
    let inner = widgets::panel(theme.block("npu", crate::theme::NPU_BOX), area, buf);
    if inner.width < 14 || inner.height < 1 {
        return;
    }
    let busy = npu.busy.or_else(|| npu.cols_ratio());
    let busy_label = if npu.busy.is_some() { "NPU" } else { "ACT" };
    let rows = [
        (
            busy_label,
            busy,
            match busy {
                Some(value) => format!("{:>5.1}%", value * 100.0),
                None => "    -".to_string(),
            },
            Gradient::Cpu,
        ),
        (
            "PWR",
            npu.power_w.map(|value| value / 30.0),
            match npu.power_w {
                Some(value) => format!("{:>5.1}W", value),
                None => "    -".to_string(),
            },
            Gradient::Temp,
        ),
        (
            "COLS",
            npu.cols_ratio(),
            match (npu.cols_used, npu.cols) {
                (Some(used), Some(total)) => format!("{:>2}/{:<2}", used, total),
                _ => "  -".to_string(),
            },
            Gradient::Available,
        ),
        (
            "CLK",
            util::ratio(
                npu.npu_mhz.map(|value| value as u64),
                npu.npu_max_mhz.map(|value| value as u64),
            ),
            util::human_freq(npu.npu_mhz),
            Gradient::Cached,
        ),
    ];
    for (row, (label, ratio, right, kind)) in rows.iter().enumerate().take(inner.height as usize) {
        widgets::meter(
            Rect {
                x: inner.x,
                y: inner.y + row as u16,
                width: inner.width,
                height: 1,
            },
            buf,
            *ratio,
            label,
            right,
            theme,
            *kind,
        );
    }
}

fn graph_box(area: Rect, buf: &mut Buffer, npu: &NpuSample, hist: &History, theme: &Theme) {
    let title = match npu.busy {
        Some(busy) => format!(
            "npu usage {} · {}",
            util::human_pct(Some(busy)).trim(),
            util::truncate(npu.label(), 24)
        ),
        None => format!(
            "npu active cols {} · {}",
            util::human_pct(npu.cols_ratio()).trim(),
            util::truncate(npu.label(), 20)
        ),
    };
    let inner = widgets::panel(theme.block(&title, crate::theme::NPU_BOX), area, buf);
    if inner.width < 10 || inner.height < 3 {
        return;
    }

    // Leave one row at the bottom for the column strip.
    let graph_height = inner.height.saturating_sub(2).max(1);
    let graph_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: graph_height,
    };
    widgets::usage_graph(
        graph_area,
        buf,
        &hist.npu.values(),
        1.0,
        theme,
        Gradient::Cpu,
        true,
    );

    let strip = Rect {
        x: inner.x,
        y: inner.bottom().saturating_sub(1),
        width: inner.width,
        height: 1,
    };
    column_strip(strip, buf, npu, theme);
}

/// One compact cell per AIE column: `C0 ███`.
fn column_strip(area: Rect, buf: &mut Buffer, npu: &NpuSample, theme: &Theme) {
    let columns = npu.cols.unwrap_or(8).max(1).min(16) as usize;
    if area.width < 8 {
        return;
    }
    let cell = (area.width as usize / columns).max(4);
    let bar_width = cell.saturating_sub(3).max(1);
    for index in 0..columns {
        let x = area.left() + (index * cell) as u16;
        if x + 3 >= area.right() {
            break;
        }
        buf.set_line(
            x,
            area.top(),
            &Line::from(Span::styled(
                format!("C{:<2}", index),
                Style::default().fg(theme.dim()),
            )),
            3,
        );
        let ratio = npu.col_busy.get(index).copied();
        if ratio.is_none() {
            let allocated = npu
                .cols_bitmap
                .map(|bitmap| bitmap & (1 << index) != 0)
                .unwrap_or(false);
            if allocated {
                buf.set_line(
                    x + 3,
                    area.top(),
                    &Line::from(Span::styled(
                        theme.full(),
                        Style::default().fg(crate::theme::GTT_ACCENT),
                    )),
                    1,
                );
            }
            continue;
        }
        widgets::meter(
            Rect {
                x: x + 3,
                y: area.top(),
                width: bar_width as u16,
                height: 1,
            },
            buf,
            ratio,
            "",
            "",
            theme,
            Gradient::Cpu,
        );
    }
}

fn stats_box(area: Rect, buf: &mut Buffer, npu: &NpuSample, hist: &History, theme: &Theme) {
    let inner = widgets::panel(theme.block("array", crate::theme::NPU_BOX), area, buf);
    if inner.width < 14 || inner.height < 4 {
        return;
    }
    let mut y = inner.y;

    let busy = npu.busy.or_else(|| npu.cols_ratio());
    let busy_label = if npu.busy.is_some() { "NPU" } else { "ACT" };
    widgets::meter(
        Rect {
            x: inner.x,
            y,
            width: inner.width,
            height: 1,
        },
        buf,
        busy,
        busy_label,
        &match busy {
            Some(value) => format!("{:>5.1}%", value * 100.0),
            None => "    -".to_string(),
        },
        theme,
        Gradient::Cpu,
    );
    y += 1;

    if y < inner.bottom() {
        let power = npu.power_w;
        widgets::meter(
            Rect {
                x: inner.x,
                y,
                width: inner.width,
                height: 1,
            },
            buf,
            power.map(|value| value / 30.0),
            "PWR",
            &match power {
                Some(value) => format!("{:>5.1}W", value),
                None => "    -".to_string(),
            },
            theme,
            Gradient::Temp,
        );
        y += 1;
    }

    if y < inner.bottom() {
        widgets::meter(
            Rect {
                x: inner.x,
                y,
                width: inner.width,
                height: 1,
            },
            buf,
            npu.cols_ratio(),
            "COLS",
            &match (npu.cols_used, npu.cols) {
                (Some(used), Some(total)) => format!("{:>2}/{:<2}", used, total),
                _ => "  -".to_string(),
            },
            theme,
            Gradient::Available,
        );
        y += 1;
    }

    if y < inner.bottom() {
        widgets::meter(
            Rect {
                x: inner.x,
                y,
                width: inner.width,
                height: 1,
            },
            buf,
            util::ratio(
                npu.npu_mhz.map(|value| value as u64),
                npu.npu_max_mhz.map(|value| value as u64),
            ),
            "CLK",
            &util::human_freq(npu.npu_mhz),
            theme,
            Gradient::Cached,
        );
        y += 1;
    }

    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(widgets::kv(
        "mode",
        npu.power_mode.clone().unwrap_or_else(|| "-".to_string()),
        theme,
    ));
    lines.push(widgets::kv(
        "tops",
        format!(
            "{} / {}",
            npu.tops_curr
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string()),
            npu.tops_max
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string())
        ),
        theme,
    ));
    lines.push(widgets::kv(
        "tasks",
        format!(
            "{} / {}",
            npu.task_curr
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string()),
            npu.task_max
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string())
        ),
        theme,
    ));
    lines.push(widgets::kv(
        "array",
        format!(
            "{}x{} · AIE {}",
            npu.cols.unwrap_or(0),
            npu.rows.unwrap_or(0),
            npu.aie_version
                .map(|(major, minor)| format!("{}.{}", major, minor))
                .unwrap_or_else(|| "?".to_string())
        ),
        theme,
    ));
    lines.push(widgets::kv("h clock", util::human_freq(npu.h_mhz), theme));
    lines.push(widgets::kv(
        "firmware",
        if npu.fw.is_empty() {
            "-".to_string()
        } else {
            npu.fw.clone()
        },
        theme,
    ));
    lines.push(widgets::kv("bdf", npu.bdf.clone(), theme));

    if let Some(core) = npu.tile_core {
        lines.push(widgets::kv("core", core.describe(), theme));
    }

    if let Some(error) = &npu.error {
        let text = if error.starts_with("open ") {
            "cannot open /dev/accel (render group?)".to_string()
        } else if error.starts_with("sensors:") {
            "sensors off: no AMD PMF NPU metrics".to_string()
        } else {
            error.clone()
        };
        lines.push(Line::from(Span::styled(
            util::truncate(&text, inner.width as usize),
            Style::default().fg(crate::theme::WARN),
        )));
    }
    if !hist.npu_clock.is_empty() {
        let clock = hist.npu_clock.values();
        let min = clock.iter().copied().fold(f64::INFINITY, f64::min);
        let max = clock.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        // A flat spark is just a blue bar; only show it when the clock moves.
        if max - min >= 10.0 {
            let peak = npu.npu_max_mhz.unwrap_or(max).max(1.0);
            let mut line = widgets::spark(
                &clock,
                inner.width.saturating_sub(4) as usize,
                peak,
                theme,
                Gradient::Cached,
            );
            line.spans
                .insert(0, Span::styled("clk ", Style::default().fg(theme.dim())));
            lines.push(line);
        }
    }

    if y < inner.bottom() {
        widgets::lines(
            Rect {
                x: inner.x,
                y: y + 1,
                width: inner.width,
                height: inner.height.saturating_sub(y - inner.y + 1),
            },
            buf,
            lines,
        );
    }
}

fn contexts_box(area: Rect, buf: &mut Buffer, snap: &Snapshot, npu: &NpuSample, theme: &Theme) {
    let active = npu
        .contexts
        .iter()
        .filter(|context| context.active())
        .count();
    let title = format!("contexts · {} · {} active", npu.contexts.len(), active);
    let inner = widgets::panel(theme.block(&title, crate::theme::NPU_BOX), area, buf);
    if inner.width < 20 || inner.height < 2 {
        return;
    }

    let header = format!(
        "{:>7} {:<18} {:>5} {:<7} {:>7} {:>9} {:>5}",
        "PID", "COMMAND", "COLS", "STATE", "HEAP", "SUBMITS", "ERR"
    );
    buf.set_line(
        inner.x,
        inner.y,
        &Line::from(Span::styled(
            util::truncate(&header, inner.width as usize),
            Style::default()
                .fg(theme.dim())
                .add_modifier(Modifier::BOLD),
        )),
        inner.width,
    );

    if npu.contexts.is_empty() {
        if inner.height > 1 {
            buf.set_line(
                inner.x,
                inner.y + 1,
                &Line::from(Span::styled(
                    "no active contexts — the NPU is idle",
                    Style::default().fg(theme.dim()),
                )),
                inner.width,
            );
        }
        return;
    }

    for (row, context) in npu
        .contexts
        .iter()
        .enumerate()
        .take(inner.height.saturating_sub(1) as usize)
    {
        let y = inner.y + 1 + row as u16;
        let comm = snap
            .procs
            .iter()
            .find(|proc| proc.pid as i64 == context.pid)
            .map(|proc| proc.comm.clone())
            .unwrap_or_else(|| "?".to_string());
        let line = context_line(context, &comm, theme);
        buf.set_line(inner.x, y, &line, inner.width);
    }
}

fn context_line(context: &NpuContext, comm: &str, theme: &Theme) -> Line<'static> {
    let state = match context.state {
        0 => "idle",
        1 => "active",
        _ => "?",
    };
    let state_style = if context.active() {
        Style::default().fg(crate::theme::GOOD)
    } else {
        Style::default().fg(theme.dim())
    };
    let error_style = if context.errors > 0 {
        Style::default().fg(crate::theme::BAD)
    } else {
        Style::default().fg(theme.dim())
    };
    Line::from(vec![
        Span::styled(
            format!("{:>7} ", context.pid),
            Style::default().fg(theme.dim()),
        ),
        Span::styled(
            format!("{:<18} ", util::truncate(comm, 18)),
            Style::default().fg(theme.strong()),
        ),
        Span::styled(
            format!("{:>5} ", context.column_span()),
            Style::default().fg(crate::theme::STRONG),
        ),
        Span::styled(format!("{:<7} ", state), state_style),
        Span::styled(
            format!("{:>7} ", util::human_bytes_compact(context.heap)),
            Style::default().fg(theme.label()),
        ),
        Span::styled(
            format!("{:>9} ", context.submissions),
            Style::default().fg(theme.dim()),
        ),
        Span::styled(format!("{:>5}", context.errors), error_style),
    ])
}
