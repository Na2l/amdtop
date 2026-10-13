//! btop-style net and disk boxes for the dashboard.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::collect::{History, Snapshot};
use crate::theme::{Gradient, Theme};
use crate::util;

use super::widgets;

/// Network box: interface name, up/down rates and sparklines.
pub fn net_box(area: Rect, buf: &mut Buffer, snap: &Snapshot, hist: &History, theme: &Theme) {
    let title = match snap.net.first() {
        Some(iface) => format!("net · {}", iface.name),
        None => "net".to_string(),
    };
    let inner = widgets::panel(theme.block(&title, crate::theme::NET_BOX), area, buf);
    if inner.width < 12 || inner.height == 0 {
        return;
    }
    let Some(iface) = snap.net.first() else {
        widgets::lines(
            inner,
            buf,
            vec![Line::from(Span::styled(
                "no interfaces",
                Style::default().fg(crate::theme::DIM),
            ))],
        );
        return;
    };

    let arrow_down = if theme.ascii { "v" } else { "▼" };
    let arrow_up = if theme.ascii { "^" } else { "▲" };
    let mut lines = vec![Line::from(vec![
        Span::styled(format!("{} ", arrow_down), Style::default().fg(crate::theme::GOOD)),
        Span::styled(
            util::human_rate(Some(iface.rx_bps)),
            Style::default()
                .fg(crate::theme::GOOD)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("   ", Style::default()),
        Span::styled(format!("{} ", arrow_up), Style::default().fg(crate::theme::CPU_ACCENT)),
        Span::styled(
            util::human_rate(Some(iface.tx_bps)),
            Style::default()
                .fg(crate::theme::CPU_ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
    ])];
    lines.push(widgets::kv(
        "total",
        format!(
            "{} in / {} out",
            util::human_bytes(iface.rx_total),
            util::human_bytes(iface.tx_total)
        ),
        theme,
    ));
    // Sparklines fill the rest of the box.
    let mut used = lines.len() as u16;
    if inner.height > used + 1 {
        let peak = hist
            .net_rx
            .max()
            .unwrap_or(0.0)
            .max(hist.net_tx.max().unwrap_or(0.0))
            .max(1024.0);
        lines.push(widgets::spark(
            &hist.net_rx.values(),
            inner.width as usize,
            peak,
            theme,
            Gradient::Download,
        ));
        used += 1;
        if inner.height > used {
            lines.push(widgets::spark(
                &hist.net_tx.values(),
                inner.width as usize,
                peak,
                theme,
                Gradient::Upload,
            ));
        }
    }
    widgets::lines(inner, buf, lines);
}

/// Disk box: per-device read/write rates and busy percentage.
pub fn disk_box(area: Rect, buf: &mut Buffer, snap: &Snapshot, hist: &History, theme: &Theme) {
    let title = match snap.disks.len() {
        0 => "disks".to_string(),
        1 => format!("disks · {}", snap.disks[0].name),
        count => format!("disks · {} devices", count),
    };
    let inner = widgets::panel(theme.block(&title, crate::theme::DISK_BOX), area, buf);
    if inner.width < 14 || inner.height == 0 {
        return;
    }
    if snap.disks.is_empty() {
        widgets::lines(
            inner,
            buf,
            vec![Line::from(Span::styled(
                "no block devices",
                Style::default().fg(crate::theme::DIM),
            ))],
        );
        return;
    }

    let mut lines = Vec::new();
    let rows_per_disk = if snap.disks.len() > 1 && inner.height >= (snap.disks.len() as u16) * 2 {
        3
    } else {
        2
    };
    for disk in snap.disks.iter().take(4) {
        lines.push(Line::from(vec![
            Span::styled(
                util::fit(&util::truncate(&disk.name, 14), 15),
                Style::default().fg(crate::theme::STRONG),
            ),
            Span::styled("R ", Style::default().fg(crate::theme::DIM)),
            Span::styled(
                util::human_rate(Some(disk.read_bps)),
                Style::default().fg(crate::theme::VRAM_ACCENT),
            ),
            Span::styled("  W ", Style::default().fg(crate::theme::DIM)),
            Span::styled(
                util::human_rate(Some(disk.write_bps)),
                Style::default().fg(crate::theme::IO_WRITE),
            ),
        ]));
        lines.push(widgets::kv(
            "busy",
            format!(
                "{:>5.1}%  {:.0} / {:.0} iops",
                disk.busy * 100.0,
                disk.read_iops,
                disk.write_iops
            ),
            theme,
        ));
        if rows_per_disk == 3 {
            let peak = hist
                .disk_read
                .max()
                .unwrap_or(0.0)
                .max(hist.disk_write.max().unwrap_or(0.0))
                .max(1024.0);
            lines.push(widgets::spark(
                &hist.disk_read.values(),
                inner.width as usize,
                peak,
                theme,
                Gradient::Cached,
            ));
        }
    }
    widgets::lines(inner, buf, lines);
}
