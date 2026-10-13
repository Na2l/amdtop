//! Process list: `top`-style table plus per-process GPU (GTT/VRAM) columns.

use std::collections::HashMap;

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Cell, Row, StatefulWidget, Table, TableState};

use crate::collect::{ProcInfo, Snapshot};
use crate::theme::{Gradient, Theme};
use crate::util;

use super::widgets;
use super::{App, SortKey};

/// Which columns fit into the available width.
#[derive(Debug, Clone, Copy)]
struct Columns {
    user: bool,
    priority: bool,
    virt: bool,
    shared: bool,
    gtt: bool,
    vram: bool,
}

impl Columns {
    fn for_width(width: u16) -> Columns {
        Columns {
            user: width >= 78,
            priority: width >= 104,
            virt: width >= 118,
            shared: width >= 132,
            gtt: width >= 96,
            vram: width >= 110,
        }
    }
}

/// Filter, sort and (optionally) tree the process list.
pub fn visible<'a>(app: &App, snap: &'a Snapshot) -> Vec<(&'a ProcInfo, usize)> {
    let filter = app.filter.to_ascii_lowercase();
    let user = app.user_filter_text().to_ascii_lowercase();
    let mut rows: Vec<&ProcInfo> = snap
        .procs
        .iter()
        .filter(|proc| {
            if app.gpu_only && !proc.has_gpu() {
                return false;
            }
            if !user.is_empty() && !proc.user.to_ascii_lowercase().contains(&user) {
                return false;
            }
            if filter.is_empty() {
                return true;
            }
            proc.comm.to_ascii_lowercase().contains(&filter)
                || proc.cmdline.to_ascii_lowercase().contains(&filter)
        })
        .collect();
    sort_rows(&mut rows, app.sort, app.sort_desc);
    if app.tree {
        build_tree(&rows, app)
    } else {
        rows.into_iter().map(|proc| (proc, 0)).collect()
    }
}

/// What the COMMAND column shows: the full command line, or the comm for
/// kernel threads whose cmdline is empty.
fn display_command(proc: &ProcInfo) -> &str {
    if proc.cmdline.is_empty() {
        &proc.comm
    } else {
        &proc.cmdline
    }
}

fn sort_rows(rows: &mut [&ProcInfo], key: SortKey, desc: bool) {
    let compare = |a: &ProcInfo, b: &ProcInfo| match key {
        SortKey::Cpu => a.cpu.partial_cmp(&b.cpu).unwrap_or(std::cmp::Ordering::Equal),
        SortKey::Pid => a.pid.cmp(&b.pid),
        SortKey::Mem => a.mem.partial_cmp(&b.mem).unwrap_or(std::cmp::Ordering::Equal),
        SortKey::Gtt => a.gtt.cmp(&b.gtt),
        SortKey::Vram => a.vram.cmp(&b.vram),
        SortKey::GpuTotal => a.gpu_total().cmp(&b.gpu_total()),
        SortKey::Time => a.time.partial_cmp(&b.time).unwrap_or(std::cmp::Ordering::Equal),
        SortKey::Name => display_command(a)
            .to_ascii_lowercase()
            .cmp(&display_command(b).to_ascii_lowercase()),
        SortKey::User => a.user.cmp(&b.user),
    };
    if desc {
        rows.sort_by(|a, b| compare(b, a));
    } else {
        rows.sort_by(|a, b| compare(a, b));
    }
}

/// Depth first process tree honouring collapsed nodes.
fn build_tree<'a>(rows: &[&'a ProcInfo], app: &App) -> Vec<(&'a ProcInfo, usize)> {
    let present: HashMap<u32, &ProcInfo> = rows.iter().map(|proc| (proc.pid, *proc)).collect();
    let mut children: HashMap<u32, Vec<&ProcInfo>> = HashMap::new();
    let mut roots: Vec<&ProcInfo> = Vec::new();
    for proc in rows {
        let parent_exists = present.contains_key(&proc.ppid) && proc.ppid != proc.pid;
        if parent_exists {
            children.entry(proc.ppid).or_default().push(proc);
        } else {
            roots.push(proc);
        }
    }
    let mut out = Vec::with_capacity(rows.len());
    let mut stack: Vec<(&ProcInfo, usize)> = roots.iter().map(|proc| (*proc, 0)).collect();
    stack.reverse();
    while let Some((proc, depth)) = stack.pop() {
        out.push((proc, depth));
        if app.collapsed.contains(&proc.pid) {
            continue;
        }
        if let Some(kids) = children.get(&proc.pid) {
            for kid in kids.iter().rev() {
                stack.push((kid, depth + 1));
            }
        }
    }
    // Guard against cycles so the list can never grow unbounded.
    out.truncate(rows.len());
    out
}

/// btop-style process box: Pid, Program, Command, Threads, User, MemB, Cpu% + GTT.
pub fn btop_box(area: Rect, buf: &mut Buffer, app: &App, snap: &Snapshot, theme: &Theme) {
    let title = format!(
        "proc · {} · sort {}{}",
        if app.tree { "tree" } else { "flat" },
        if app.sort_desc { "-" } else { "+" },
        app.sort.label()
    );
    let inner = widgets::panel(theme.block(&title, crate::theme::PROC_BOX), area, buf);
    if inner.width < 24 || inner.height < 2 {
        return;
    }
    let rows = visible(app, snap);
    let width = inner.width;
    let show_program = width >= 58;
    let show_mem = width >= 68;
    let show_gtt = width >= 78;
    let show_user = width >= 92;
    let show_threads = width >= 112;

    // btop column order, dropping the least important from the right.
    let mut header: Vec<Cell> = vec![Cell::from("Pid")];
    let mut widths: Vec<Constraint> = vec![Constraint::Length(7)];
    if show_program {
        header.push(Cell::from("Program"));
        widths.push(Constraint::Length(17));
    }
    header.push(Cell::from("Cpu%"));
    widths.push(Constraint::Length(5));
    if show_mem {
        header.push(Cell::from("MemB"));
        widths.push(Constraint::Length(7));
    }
    if show_gtt {
        header.push(Cell::from("GTT"));
        widths.push(Constraint::Length(7));
    }
    if show_user {
        header.push(Cell::from("User"));
        widths.push(Constraint::Length(8));
    }
    if show_threads {
        header.push(Cell::from("Thr"));
        widths.push(Constraint::Length(4));
    }
    header.push(Cell::from("Command"));
    widths.push(Constraint::Min(10));

    let visible_height = inner.height.saturating_sub(1) as usize;
    let selected = if rows.is_empty() {
        0
    } else {
        app.selected.min(rows.len() - 1)
    };
    let offset = selected.saturating_sub(visible_height.saturating_sub(1));
    let body: Vec<Row> = rows
        .iter()
        .skip(offset)
        .take(visible_height)
        .map(|(proc, depth)| {
            let mut cells: Vec<Cell> = vec![Cell::from(proc.pid.to_string())];
            if show_program {
                cells.push(Cell::from(util::truncate(&proc.comm, 17)));
            }
            cells.push(
                Cell::from(format!("{:.1}", proc.cpu))
                    .style(widgets::load_style((proc.cpu / 100.0).min(1.0), theme, Gradient::Process)),
            );
            if show_mem {
                cells.push(Cell::from(format!("{:.0}", proc.rss as f64 / (1024.0 * 1024.0))));
            }
            if show_gtt {
                cells.push(
                    Cell::from(if proc.gtt > 0 {
                        util::human_bytes_compact(proc.gtt)
                    } else {
                        "-".to_string()
                    })
                    .style(if proc.gtt > 0 {
                        Style::default()
                            .fg(crate::theme::GTT_ACCENT)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(crate::theme::DIM)
                    }),
                );
            }
            if show_user {
                cells.push(
                    Cell::from(util::truncate(&proc.user, 8))
                        .style(Style::default().fg(crate::theme::PROC_MISC)),
                );
            }
            if show_threads {
                cells.push(
                    Cell::from(proc.threads.to_string())
                        .style(Style::default().fg(crate::theme::PROC_MISC)),
                );
            }
            let depth = *depth;
            let indent = if depth > 0 {
                format!("{}{} ", "  ".repeat(depth.saturating_sub(1)), theme.arrow())
            } else {
                String::new()
            };
            let command = if proc.cmdline.is_empty() {
                format!("[{}]", proc.comm)
            } else {
                proc.cmdline.clone()
            };
            cells.push(Cell::from(format!("{}{}", indent, command)));
            Row::new(cells)
        })
        .collect();

    let table = Table::new(body, widths)
        .header(Row::new(header).style(Style::default().fg(crate::theme::DIM)))
        .row_highlight_style(
            Style::default()
                .fg(crate::theme::SELECTED_FG)
                .bg(crate::theme::SELECTED_BG)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">");
    let mut state = TableState::default()
        .with_offset(offset)
        .with_selected(Some(selected));
    StatefulWidget::render(table, inner, buf, &mut state);
}

/// Full PROC tab.
pub fn full(area: Rect, buf: &mut Buffer, app: &App, snap: &Snapshot, theme: &Theme) {
    let title = format!(
        "PROCESSES {}{}{} · sort {}{} · [e]tree {} · [G]gpu-only {} · [f]ilter {}",
        if app.tree { "[tree] " } else { "" },
        if app.gpu_only { "gpu-only " } else { "" },
        if app.filter.is_empty() && app.user_filter_text().is_empty() {
            String::new()
        } else {
            format!(
                "filter {:?} ",
                if app.filter.is_empty() {
                    app.user_filter_text()
                } else {
                    &app.filter
                }
            )
        },
        if app.sort_desc { "-" } else { "+" },
        app.sort.label(),
        if app.tree { "on" } else { "off" },
        if app.gpu_only { "on" } else { "off" },
        if app.filter_active { "…" } else { "" }
    );
    let inner = widgets::panel(theme.block(&title, crate::theme::PROC_BOX), area, buf);
    let rows = visible(app, snap);
    let columns = Columns::for_width(inner.width);

    let mut header_cells: Vec<Cell> = vec![Cell::from("PID")];
    let mut widths: Vec<Constraint> = vec![Constraint::Length(6)];
    let push = |label: &'static str, width: u16, cells: &mut Vec<Cell>, widths: &mut Vec<Constraint>| {
        cells.push(Cell::from(label));
        widths.push(Constraint::Length(width));
    };
    if columns.user {
        push("USER", 8, &mut header_cells, &mut widths);
    }
    if columns.priority {
        push("PR", 3, &mut header_cells, &mut widths);
        push("NI", 3, &mut header_cells, &mut widths);
    }
    if columns.virt {
        push("VIRT", 6, &mut header_cells, &mut widths);
    }
    push("RES", 6, &mut header_cells, &mut widths);
    if columns.shared {
        push("SHR", 6, &mut header_cells, &mut widths);
    }
    push("S", 1, &mut header_cells, &mut widths);
    push("%CPU", 5, &mut header_cells, &mut widths);
    push("%MEM", 5, &mut header_cells, &mut widths);
    if columns.gtt {
        push("GTT", 7, &mut header_cells, &mut widths);
    }
    if columns.vram {
        push("VRAM", 7, &mut header_cells, &mut widths);
    }
    push("TIME+", 8, &mut header_cells, &mut widths);
    widths.push(Constraint::Min(10));
    header_cells.push(Cell::from("COMMAND"));

    let visible_height = inner.height.saturating_sub(1) as usize;
    let selected = if rows.is_empty() { 0 } else { app.selected.min(rows.len() - 1) };
    let offset = selected.saturating_sub(visible_height.saturating_sub(1));

    let body: Vec<Row> = rows
        .iter()
        .skip(offset)
        .take(visible_height)
        .map(|(proc, depth)| row_for(*proc, *depth, columns, theme))
        .collect();

    let table = Table::new(body, widths)
        .header(Row::new(header_cells).style(Style::default().fg(crate::theme::DIM)))
        .row_highlight_style(
            Style::default()
                .fg(crate::theme::SELECTED_FG)
                .bg(crate::theme::SELECTED_BG)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");
    let mut state = TableState::default().with_offset(offset).with_selected(Some(selected));
    StatefulWidget::render(table, inner, buf, &mut state);
}

fn row_for<'a>(proc: &ProcInfo, depth: usize, columns: Columns, theme: &Theme) -> Row<'a> {
    let mut cells: Vec<Cell> = vec![Cell::from(proc.pid.to_string())];
    if columns.user {
        cells.push(Cell::from(util::truncate(&proc.user, 8)));
    }
    if columns.priority {
        cells.push(Cell::from(format!("{}", proc.priority)));
        cells.push(Cell::from(format!("{}", proc.nice)));
    }
    if columns.virt {
        cells.push(Cell::from(util::human_bytes_compact(proc.virt)));
    }
    cells.push(Cell::from(util::human_bytes_compact(proc.rss)));
    if columns.shared {
        cells.push(Cell::from(util::human_bytes_compact(proc.shared)));
    }
    cells.push(Cell::from(proc.state.to_string()));
    cells.push(
        Cell::from(format!("{:.1}", proc.cpu)).style(widgets::load_style((proc.cpu / 100.0).min(1.0), theme, Gradient::Process)),
    );
    cells.push(Cell::from(format!("{:.1}", proc.mem)));
    if columns.gtt {
        cells.push(Cell::from(
            if proc.gtt > 0 {
                util::human_bytes_compact(proc.gtt)
            } else {
                "-".to_string()
            },
        ).style(if proc.gtt > 0 {
            Style::default().fg(crate::theme::GTT_ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(crate::theme::DIM)
        }));
    }
    if columns.vram {
        cells.push(Cell::from(
            if proc.vram > 0 {
                util::human_bytes_compact(proc.vram)
            } else {
                "-".to_string()
            },
        ).style(if proc.vram > 0 {
            Style::default().fg(crate::theme::VRAM_ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(crate::theme::DIM)
        }));
    }
    cells.push(Cell::from(util::human_cputime(proc.time)));
    let indent = if depth > 0 {
        format!("{}{} ", "  ".repeat(depth.saturating_sub(1)), theme.arrow())
    } else {
        String::new()
    };
    let command = if proc.cmdline.is_empty() {
        format!("[{}]", proc.comm)
    } else {
        proc.cmdline.clone()
    };
    cells.push(Cell::from(format!("{}{}", indent, command)));
    Row::new(cells)
}

/// The pid currently under the cursor, used by the kill prompt.
pub fn selected_pid(app: &App, snap: &Snapshot) -> Option<u32> {
    let rows = visible(app, snap);
    if rows.is_empty() {
        return None;
    }
    let index = app.selected.min(rows.len() - 1);
    Some(rows[index].0.pid)
}
