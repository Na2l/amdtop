//! Low level drawing helpers: meters, graphs and sparklines.
//!
//! Colours follow btop: meter cells are painted with a gradient *along the bar*
//! (so the ramp is visible up to the fill point), while graph columns are painted
//! by their own height. The unfilled part of a meter uses btop's `meter_bg`.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Widget};

use crate::theme::{Gradient, Theme};

/// Braille dot bit positions: `(column, row) -> bit`.
const BRAILLE_BITS: [[u16; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];

/// Write one symbol plus foreground colour, ignoring out of bounds coordinates.
/// Backgrounds are never set, so the terminal's own background shows through.
fn put_cell(buf: &mut Buffer, x: u16, y: u16, symbol: &str, color: Color) {
    if let Some(cell) = buf.cell_mut((x, y)) {
        cell.set_symbol(symbol);
        cell.set_style(Style::default().fg(color));
    }
}

/// Write plain text with a foreground colour.
fn put_text(buf: &mut Buffer, x: u16, y: u16, text: &str, color: Color, limit: u16) {
    let mut col = x;
    for ch in text.chars() {
        if col >= limit {
            break;
        }
        put_cell(buf, col, y, &ch.to_string(), color);
        col += 1;
    }
}

/// btop meter: `label ███████────── value`, gradient along the bar.
pub fn meter(
    area: Rect,
    buf: &mut Buffer,
    ratio: Option<f64>,
    label: &str,
    right: &str,
    theme: &Theme,
    kind: Gradient,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let y = area.top();
    let mut x = area.left();
    let limit = area.right();
    if !label.is_empty() {
        let room = limit.saturating_sub(x) as usize;
        let shown = if label.chars().count() > room {
            crate::util::truncate(label, room)
        } else {
            label.to_string()
        };
        put_text(buf, x, y, &shown, theme.label(), limit);
        x += shown.chars().count() as u16;
        x += 1;
    }
    let right_len = right.chars().count() as u16;
    let bar_width = limit.saturating_sub(x).saturating_sub(right_len + 1) as usize;
    let ratio = ratio.unwrap_or(0.0);
    let filled = ((ratio.clamp(0.0, 1.0)) * bar_width as f64).round() as usize;
    for index in 0..bar_width {
        let position = if bar_width > 1 {
            index as f64 / (bar_width - 1) as f64
        } else {
            0.0
        };
        if index < filled {
            put_cell(buf, x + index as u16, y, theme.full(), theme.grad(kind, position));
        } else {
            put_cell(buf, x + index as u16, y, theme.empty(), theme.meter_bg());
        }
    }
    let start = x + bar_width as u16 + 1;
    if start < limit {
        let text = crate::util::fit_right(right, (limit - start) as usize);
        // btop paints the percentage with the same gradient as the meter.
        put_text(buf, start, y, &text, theme.grad(kind, ratio), limit);
    }
}

/// A single bar split into coloured segments (unified memory view).
pub fn stacked(area: Rect, buf: &mut Buffer, segments: &[(f64, Color)], theme: &Theme) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let total: f64 = segments.iter().map(|(value, _)| value.max(0.0)).sum();
    let width = area.width as usize;
    let mut cells: Vec<Color> = Vec::with_capacity(width);
    if total > 0.0 {
        for (value, color) in segments {
            let count = ((value.max(0.0) / total) * width as f64).round() as usize;
            for _ in 0..count {
                if cells.len() < width {
                    cells.push(*color);
                }
            }
        }
    }
    for y in 0..area.height {
        for x in 0..width {
            match cells.get(x) {
                Some(color) => put_cell(
                    buf,
                    area.left() + x as u16,
                    area.top() + y,
                    theme.full(),
                    *color,
                ),
                None => put_cell(
                    buf,
                    area.left() + x as u16,
                    area.top() + y,
                    theme.empty(),
                    theme.meter_bg(),
                ),
            }
        }
    }
}

/// One row sparkline, each column coloured by its own value.
pub fn spark(values: &[f64], width: usize, vmax: f64, theme: &Theme, kind: Gradient) -> Line<'static> {
    let offset = width.saturating_sub(values.len());
    let start = values.len().saturating_sub(width);
    let mut spans = Vec::new();
    for index in 0..width {
        let value = if index < offset {
            0.0
        } else {
            values[start + (index - offset)]
        };
        let ratio = if vmax > 0.0 {
            crate::util::clamp(value / vmax, 0.0, 1.0)
        } else {
            0.0
        };
        let eighths = (ratio * 8.0).round() as usize;
        let color = if eighths == 0 {
            theme.meter_bg()
        } else {
            theme.grad(kind, ratio)
        };
        spans.push(Span::styled(theme.eighth(eighths), Style::default().fg(color)));
    }
    Line::from(spans)
}

/// Draw a graph: braille when the locale supports it, eighth blocks otherwise.
pub fn graph(area: Rect, buf: &mut Buffer, values: &[f64], vmax: f64, theme: &Theme, kind: Gradient) {
    if theme.braille_ok() {
        braille(area, buf, values, vmax, theme, kind);
    } else {
        blocks(area, buf, values, vmax, theme, kind);
    }
}

/// Histogram area: mirrored (centred) or growing up from the bottom.
pub fn usage_graph(
    area: Rect,
    buf: &mut Buffer,
    values: &[f64],
    vmax: f64,
    theme: &Theme,
    kind: Gradient,
    mirrored: bool,
) {
    if !mirrored {
        graph(area, buf, values, vmax, theme, kind);
    } else if theme.braille_ok() {
        braille_mirror(area, buf, values, vmax, theme, kind);
    } else {
        blocks_mirror(area, buf, values, vmax, theme, kind);
    }
}

/// Mirrored column graph for terminals without braille (Linux console).
pub fn blocks_mirror(area: Rect, buf: &mut Buffer, values: &[f64], vmax: f64, theme: &Theme, kind: Gradient) {
    if area.width == 0 || area.height == 0 || vmax <= 0.0 {
        return;
    }
    let height = area.height as i32;
    let total = height * 8;
    let centre = total / 2;
    let width = area.width as usize;
    let offset = width.saturating_sub(values.len());
    let start = values.len().saturating_sub(width);
    for col in 0..width {
        let value = if col < offset {
            0.0
        } else {
            values[start + (col - offset)]
        };
        let ratio = crate::util::clamp(value / vmax, 0.0, 1.0);
        let eighths = (ratio * total as f64).round() as i32;
        let (low, high) = if eighths == 0 {
            (centre, centre + 1)
        } else {
            let half = eighths / 2;
            let low = centre - half;
            (low, (low + eighths).min(total))
        };
        let color = theme.grad(kind, ratio);
        for row in 0..height {
            let row_low = (height - 1 - row) * 8;
            let row_high = row_low + 8;
            let overlap = (high.min(row_high) - low.max(row_low)).max(0);
            if overlap > 0 {
                put_cell(
                    buf,
                    area.left() + col as u16,
                    area.top() + row as u16,
                    theme.eighth(overlap as usize),
                    color,
                );
            } else if low.max(row_low) <= centre && centre < row_high.min(high.max(centre + 1)) {
                put_cell(
                    buf,
                    area.left() + col as u16,
                    area.top() + row as u16,
                    theme.eighth(0),
                    theme.graph_text(),
                );
            } else {
                put_cell(
                    buf,
                    area.left() + col as u16,
                    area.top() + row as u16,
                    " ",
                    theme.meter_bg(),
                );
            }
        }
    }
}

/// Braille line graph, two dots wide and four dots tall per cell.
pub fn braille(area: Rect, buf: &mut Buffer, values: &[f64], vmax: f64, theme: &Theme, kind: Gradient) {
    if area.width == 0 || area.height == 0 || vmax <= 0.0 {
        return;
    }
    let cols = area.width as usize * 2;
    let rows = area.height as usize;
    let dot_rows = rows * 4;
    let mut grid = vec![vec![0u16; cols]; rows];
    let mut levels: Vec<usize> = Vec::with_capacity(cols);
    let offset = cols.saturating_sub(values.len());
    let start = values.len().saturating_sub(cols);
    for index in 0..cols {
        let value = if index < offset {
            0.0
        } else {
            values[start + (index - offset)]
        };
        let level = ((crate::util::clamp(value, 0.0, vmax) / vmax) * dot_rows as f64).round() as usize;
        levels.push(level);
        let cell = index / 2;
        let sub = index % 2;
        if cell >= area.width as usize {
            break;
        }
        // Every column carries at least a baseline dot, which is btop's dotted
        // zero line: the graph is visible from the first frame instead of showing
        // only the handful of samples collected so far.
        for dot in 0..level.max(1) {
            let row = rows - 1 - dot / 4;
            if row < grid.len() {
                grid[row][index] |= BRAILLE_BITS[sub][dot % 4];
            }
        }
    }
    paint_braille(area, buf, &grid, &levels, dot_rows, theme, kind);
}

/// Paint a braille grid: colour by peak height, blank where there is no history.
fn paint_braille(
    area: Rect,
    buf: &mut Buffer,
    grid: &[Vec<u16>],
    levels: &[usize],
    dot_rows: usize,
    theme: &Theme,
    kind: Gradient,
) {
    for row in 0..area.height as usize {
        for col in 0..area.width as usize {
            let left = grid[row].get(col * 2).copied().unwrap_or(0);
            let right = grid[row].get(col * 2 + 1).copied().unwrap_or(0);
            let symbol = char::from_u32(0x2800 + left as u32 + right as u32).unwrap_or(' ');
            let peak = levels
                .get(col * 2)
                .copied()
                .unwrap_or(0)
                .max(levels.get(col * 2 + 1).copied().unwrap_or(0));
            // Baseline-only columns use btop's `graph_text`, not the gradient.
            let color = if peak == 0 {
                theme.graph_text()
            } else {
                theme.grad(kind, peak as f64 / dot_rows as f64)
            };
            put_cell(
                buf,
                area.left() + col as u16,
                area.top() + row as u16,
                &symbol.to_string(),
                color,
            );
        }
    }
}

/// Mirrored graph: the baseline runs through the vertical middle and the value
/// grows symmetrically above *and* below it, the way btop draws its graphs.
pub fn braille_mirror(area: Rect, buf: &mut Buffer, values: &[f64], vmax: f64, theme: &Theme, kind: Gradient) {
    if area.width == 0 || area.height == 0 || vmax <= 0.0 {
        return;
    }
    let cols = area.width as usize * 2;
    let rows = area.height as usize;
    let dot_rows = rows * 4;
    let centre = dot_rows / 2;
    let mut grid = vec![vec![0u16; cols]; rows];
    let mut levels: Vec<usize> = Vec::with_capacity(cols);
    let offset = cols.saturating_sub(values.len());
    let start = values.len().saturating_sub(cols);
    for index in 0..cols {
        let value = if index < offset {
            0.0
        } else {
            values[start + (index - offset)]
        };
        let level = ((crate::util::clamp(value, 0.0, vmax) / vmax) * dot_rows as f64).round() as usize;
        levels.push(level);
        let cell = index / 2;
        let sub = index % 2;
        if cell >= area.width as usize {
            break;
        }
        // Dots counted from the bottom, centred on the middle line.
        let (low, high) = if level == 0 {
            (centre.saturating_sub(1), centre)
        } else {
            let half = level / 2;
            let low = centre.saturating_sub(half);
            (low, (low + level).min(dot_rows))
        };
        for dot in low..high.max(low + 1) {
            let row = rows - 1 - dot / 4;
            if row < grid.len() {
                grid[row][index] |= BRAILLE_BITS[sub][dot % 4];
            }
        }
    }
    paint_braille(area, buf, &grid, &levels, dot_rows, theme, kind);
}

/// Column graph built from vertical eighth blocks; ASCII safe.
pub fn blocks(area: Rect, buf: &mut Buffer, values: &[f64], vmax: f64, theme: &Theme, kind: Gradient) {
    if area.width == 0 || area.height == 0 || vmax <= 0.0 {
        return;
    }
    let width = area.width as usize;
    let offset = width.saturating_sub(values.len());
    let start = values.len().saturating_sub(width);
    for col in 0..width {
        let value = if col < offset {
            0.0
        } else {
            values[start + (col - offset)]
        };
        let ratio = crate::util::clamp(value / vmax, 0.0, 1.0);
        let eighths = (ratio * area.height as f64 * 8.0).round() as i32;
        let color = theme.grad(kind, ratio);
        for row in 0..area.height {
            let from_bottom = area.height as i32 - 1 - row as i32;
            let remaining = eighths - from_bottom * 8;
            if remaining <= 0 {
                // Bottom row keeps a mark so a flat graph is still visible.
                let baseline = row + 1 == area.height;
                put_cell(
                    buf,
                    area.left() + col as u16,
                    area.top() + row,
                    if baseline { "_" } else { " " },
                    if baseline {
                        theme.graph_text()
                    } else {
                        theme.meter_bg()
                    },
                );
            } else {
                put_cell(
                    buf,
                    area.left() + col as u16,
                    area.top() + row,
                    theme.eighth(remaining.min(8) as usize),
                    color,
                );
            }
        }
    }
}

/// `label  value` row with a dim label.
pub fn kv<'a>(label: &str, value: String, theme: &Theme) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("{:<10} ", label), Style::default().fg(theme.label())),
        Span::styled(value, Style::default().fg(theme.strong())),
    ])
}

/// `label value` with a caller supplied value style.
pub fn kv_styled<'a>(label: &str, value: String, style: Style, theme: &Theme) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("{:<10} ", label), Style::default().fg(theme.label())),
        Span::styled(value, style),
    ])
}

/// Small legend row: `■ label  ■ label`.
pub fn legend(items: &[(Color, String)], theme: &Theme) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, (color, label)) in items.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(theme.full(), Style::default().fg(*color)));
        spans.push(Span::styled(
            format!(" {}", label),
            Style::default().fg(theme.label()),
        ));
    }
    Line::from(spans)
}

/// Percentage text coloured by a btop gradient.
pub fn load_style(ratio: f64, theme: &Theme, kind: Gradient) -> Style {
    Style::default()
        .fg(theme.grad(kind, ratio))
        .add_modifier(if ratio > 0.9 {
            Modifier::BOLD
        } else {
            Modifier::empty()
        })
}

/// Render a bordered block and return its inner area.
pub fn panel(block: Block, area: Rect, buf: &mut Buffer) -> Rect {
    let inner = block.inner(area);
    block.render(area, buf);
    inner
}

/// Render text lines into an area, clipping to what fits.
pub fn lines(area: Rect, buf: &mut Buffer, lines: Vec<Line<'static>>) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    Paragraph::new(lines).render(area, buf);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;

    fn render(values: &[f64], width: u16, height: u16) -> String {
        let area = Rect {
            x: 0,
            y: 0,
            width,
            height,
        };
        let mut buf = Buffer::empty(area);
        let theme = Theme::new(false, true);
        graph(area, &mut buf, values, 1.0, &theme, Gradient::Cpu);
        let mut out = String::new();
        for y in 0..height {
            for x in 0..width {
                if let Some(cell) = buf.cell((x, y)) {
                    out.push_str(cell.symbol());
                }
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn idle_graph_still_draws_a_baseline() {
        // Regression: a flat 0% graph used to render as blank, which made the CPU
        // and GPU histograms look like they were missing entirely.
        let text = render(&[0.0; 64], 24, 4);
        let glyphs: Vec<char> = text.chars().filter(|c| *c != '\n').collect();
        assert!(
            glyphs.iter().any(|c| *c != ' ' && *c != '\u{2800}'),
            "idle graph rendered completely blank:\n{}",
            text
        );
        assert!(text.contains('⠉'), "no baseline dots in:\n{}", text);
    }

    #[test]
    fn busy_graph_is_taller_than_idle() {
        let idle = render(&[0.0; 64], 24, 4);
        let busy = render(&[1.0; 64], 24, 4);
        let ink = |text: &str| {
            text.chars()
                .filter(|c| !matches!(*c, ' ' | '\n' | '\u{2800}'))
                .count()
        };
        assert!(
            ink(&busy) > ink(&idle) * 2,
            "busy graph should fill more cells than idle\nidle:\n{}\nbusy:\n{}",
            idle,
            busy
        );
    }

    #[test]
    fn meter_shows_empty_dashes_when_idle() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 30,
            height: 1,
        };
        let mut buf = Buffer::empty(area);
        let theme = Theme::new(false, true);
        meter(area, &mut buf, Some(0.0), "CPU", "0.0%", &theme, Gradient::Cpu);
        let mut text = String::new();
        for x in 0..area.width {
            if let Some(cell) = buf.cell((x, 0)) {
                text.push_str(cell.symbol());
            }
        }
        assert!(text.starts_with("CPU"), "label missing: {:?}", text);
        assert!(text.contains('─'), "empty meter has no dashes: {:?}", text);
        assert!(text.trim_end().ends_with("0.0%"), "value missing: {:?}", text);
    }
}

#[cfg(test)]
mod mirror_tests {
    use super::*;
    use crate::theme::Theme;

    fn render_mirror(values: &[f64], width: u16, height: u16) -> Vec<String> {
        let area = Rect {
            x: 0,
            y: 0,
            width,
            height,
        };
        let mut buf = Buffer::empty(area);
        let theme = Theme::new(false, true);
        braille_mirror(area, &mut buf, values, 1.0, &theme, Gradient::Cpu);
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "))
                    .collect::<String>()
            })
            .collect()
    }

    fn ink(row: &str) -> usize {
        row.chars()
            .filter(|c| !matches!(*c, ' ' | '\u{2800}'))
            .count()
    }

    #[test]
    fn mirrored_baseline_sits_in_the_middle() {
        let rows = render_mirror(&[0.0; 64], 16, 7);
        let middle = rows.len() / 2;
        assert!(ink(&rows[middle]) > 0, "no baseline on the middle row");
        // The middle line is where the dots are; top and bottom stay clear.
        let above: usize = rows[..middle].iter().map(|r| ink(r)).sum();
        let below: usize = rows[middle + 1..].iter().map(|r| ink(r)).sum();
        assert!(above <= 1 && below <= 1, "baseline is not centred: {:?}", rows);
    }

    #[test]
    fn mirrored_graph_grows_both_ways() {
        let rows = render_mirror(&[1.0; 64], 16, 7);
        let middle = rows.len() / 2;
        let above: usize = rows[..middle].iter().map(|r| ink(r)).sum();
        let below: usize = rows[middle + 1..].iter().map(|r| ink(r)).sum();
        assert!(above > 0, "full load did not grow above the middle");
        assert!(below > 0, "full load did not grow below the middle");
    }
}
