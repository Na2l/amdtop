//! UI state, key bindings and the overall frame layout.

pub mod cpu;
pub mod gpu;
pub mod mem;
pub mod proc;
pub mod system;
pub mod widgets;

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::layout::Margin;
use ratatui::widgets::{Clear, Paragraph, Widget};
use ratatui::Frame;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::args::Args;
use crate::collect::{History, Snapshot};
use crate::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Dash,
    Cpu,
    Gpu,
    Mem,
    Proc,
    Io,
}

impl Tab {
    pub const ALL: [Tab; 6] = [Tab::Dash, Tab::Cpu, Tab::Gpu, Tab::Mem, Tab::Proc, Tab::Io];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Dash => "Dash",
            Tab::Cpu => "CPU",
            Tab::Gpu => "GPU",
            Tab::Mem => "MEM",
            Tab::Proc => "PROC",
            Tab::Io => "IO",
        }
    }

    pub fn from_name(name: &str) -> Option<Tab> {
        match name {
            "dash" | "0" => Some(Tab::Dash),
            "cpu" | "1" => Some(Tab::Cpu),
            "gpu" | "2" => Some(Tab::Gpu),
            "mem" | "3" => Some(Tab::Mem),
            "proc" | "process" | "4" => Some(Tab::Proc),
            "io" | "net" | "disks" | "6" => Some(Tab::Io),
            _ => None,
        }
    }
}

/// The four memory views requested for Strix Halo: CPU RAM, GPU memory, TTM
/// internals and a unified physical-RAM breakdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemView {
    Cpu,
    Gpu,
    Ttm,
    Unified,
}

impl MemView {
    pub const ALL: [MemView; 4] = [MemView::Cpu, MemView::Gpu, MemView::Ttm, MemView::Unified];

    pub fn label(self) -> &'static str {
        match self {
            MemView::Cpu => "CPU RAM",
            MemView::Gpu => "GPU (VRAM+GTT)",
            MemView::Ttm => "TTM",
            MemView::Unified => "Unified",
        }
    }

    pub fn from_name(name: &str) -> Option<MemView> {
        match name {
            "cpu" | "1" => Some(MemView::Cpu),
            "gpu" | "2" => Some(MemView::Gpu),
            "ttm" | "3" => Some(MemView::Ttm),
            "unified" | "4" => Some(MemView::Unified),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Cpu,
    Pid,
    Mem,
    Gtt,
    Vram,
    GpuTotal,
    Time,
    Name,
    User,
}

impl SortKey {
    pub fn label(self) -> &'static str {
        match self {
            SortKey::Cpu => "%CPU",
            SortKey::Pid => "PID",
            SortKey::Mem => "%MEM",
            SortKey::Gtt => "GTT",
            SortKey::Vram => "VRAM",
            SortKey::GpuTotal => "GPU",
            SortKey::Time => "TIME+",
            SortKey::Name => "COMMAND",
            SortKey::User => "USER",
        }
    }
}

/// What a key press asks the main loop to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    None,
    Redraw,
    SetInterval(u64),
    Quit,
}

pub struct App {
    pub tab: Tab,
    pub mem_view: MemView,
    pub interval_ms: u64,
    pub paused: bool,
    pub help: bool,
    pub quit: bool,
    pub sort: SortKey,
    pub sort_desc: bool,
    pub filter: String,
    pub filter_active: bool,
    pub user_filter: String,
    pub tree: bool,
    pub collapsed: std::collections::HashSet<u32>,
    pub gpu_only: bool,
    pub selected: usize,
    pub cpu_scroll: usize,
    pub message: Option<String>,
    pub kill_pending: bool,
    pub kill_target: Option<u32>,
    /// True when running against the simulated Strix Halo backend.
    pub sim: bool,
    /// Draw usage histograms centred on their middle line (btop style).
    pub mirror_graphs: bool,
}

impl App {
    pub fn new(args: &Args) -> App {
        App {
            tab: args
                .tab
                .as_deref()
                .and_then(Tab::from_name)
                .unwrap_or(Tab::Dash),
            mem_view: args
                .mem_view
                .as_deref()
                .and_then(MemView::from_name)
                .unwrap_or(MemView::Unified),
            interval_ms: args.interval_ms,
            paused: false,
            help: false,
            quit: false,
            sort: SortKey::Cpu,
            sort_desc: true,
            filter: String::new(),
            filter_active: false,
            user_filter: String::new(),
            tree: false,
            collapsed: std::collections::HashSet::new(),
            gpu_only: args.gpu_only,
            selected: 0,
            cpu_scroll: 0,
            message: None,
            kill_pending: false,
            kill_target: None,
            sim: false,
            mirror_graphs: true,
        }
    }

    pub fn on_key(&mut self, key: &KeyEvent) -> Action {
        // Text entry for the filters takes priority.
        if self.filter_active {
            return match key.code {
                KeyCode::Esc => {
                    self.filter_active = false;
                    self.filter.clear();
                    self.user_filter.clear();
                    self.selected = 0;
                    Action::Redraw
                }
                KeyCode::Enter => {
                    self.filter_active = false;
                    Action::Redraw
                }
                KeyCode::Backspace => {
                    if self.user_filter_active() {
                        self.user_filter.pop();
                    } else {
                        self.filter.pop();
                    }
                    self.selected = 0;
                    Action::Redraw
                }
                KeyCode::Char(value) => {
                    if self.user_filter_active() {
                        self.user_filter.push(value);
                    } else {
                        self.filter.push(value);
                    }
                    self.selected = 0;
                    Action::Redraw
                }
                _ => Action::None,
            };
        }
        if self.help {
            self.help = false;
            return Action::Redraw;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return Action::Quit;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => {
                self.quit = true;
                Action::Quit
            }
            KeyCode::Char('?') | KeyCode::Char('h') => {
                self.help = true;
                Action::Redraw
            }
            KeyCode::Tab => {
                let index = Tab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0);
                self.tab = Tab::ALL[(index + 1) % Tab::ALL.len()];
                Action::Redraw
            }
            KeyCode::Char('1') => self.set_tab(Tab::Dash),
            KeyCode::Char('2') => self.set_tab(Tab::Cpu),
            KeyCode::Char('3') => self.set_tab(Tab::Gpu),
            KeyCode::Char('4') => self.set_tab(Tab::Mem),
            KeyCode::Char('5') => self.set_tab(Tab::Proc),
            KeyCode::Char('6') => self.set_tab(Tab::Io),
            KeyCode::Char('m') => {
                let index = MemView::ALL.iter().position(|v| *v == self.mem_view).unwrap_or(0);
                self.mem_view = MemView::ALL[(index + 1) % MemView::ALL.len()];
                self.tab = Tab::Mem;
                Action::Redraw
            }
            KeyCode::Char(' ') => {
                self.paused = !self.paused;
                Action::Redraw
            }
            KeyCode::Char('+') | KeyCode::Char('=') => {
                self.interval_ms = (self.interval_ms + 250).min(10_000);
                Action::SetInterval(self.interval_ms)
            }
            KeyCode::Char('-') => {
                self.interval_ms = self.interval_ms.saturating_sub(250).max(100);
                Action::SetInterval(self.interval_ms)
            }
            KeyCode::Char('f') => {
                self.filter_active = true;
                self.filter.clear();
                self.user_filter.clear();
                Action::Redraw
            }
            KeyCode::Char('/') => {
                self.filter_active = true;
                self.filter.clear();
                self.user_filter.clear();
                Action::Redraw
            }
            KeyCode::Char('e') => {
                self.tree = !self.tree;
                Action::Redraw
            }
            KeyCode::Char('g') => self.toggle_sort(SortKey::Gtt),
            KeyCode::Char('v') => self.toggle_sort(SortKey::Vram),
            KeyCode::Char('c') => self.toggle_sort(SortKey::Cpu),
            KeyCode::Char('p') => self.toggle_sort(SortKey::Pid),
            KeyCode::Char('n') => self.toggle_sort(SortKey::Mem),
            KeyCode::Char('t') => self.toggle_sort(SortKey::Time),
            KeyCode::Char('u') => self.toggle_sort(SortKey::User),
            KeyCode::Char('N') => self.toggle_sort(SortKey::Name),
            KeyCode::Char('r') => self.toggle_sort(SortKey::GpuTotal),
            KeyCode::Char('G') => {
                self.gpu_only = !self.gpu_only;
                Action::Redraw
            }
            KeyCode::Char('M') => {
                self.mirror_graphs = !self.mirror_graphs;
                Action::Redraw
            }
            KeyCode::Char('K') => {
                self.kill_pending = true;
                Action::Redraw
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.tab == Tab::Cpu {
                    self.cpu_scroll = self.cpu_scroll.saturating_sub(1);
                } else {
                    self.selected = self.selected.saturating_sub(1);
                }
                Action::Redraw
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.tab == Tab::Cpu {
                    self.cpu_scroll += 1;
                } else {
                    self.selected += 1;
                }
                Action::Redraw
            }
            KeyCode::PageUp => {
                self.selected = self.selected.saturating_sub(20);
                Action::Redraw
            }
            KeyCode::PageDown => {
                self.selected += 20;
                Action::Redraw
            }
            KeyCode::Home => {
                self.selected = 0;
                Action::Redraw
            }
            _ => Action::None,
        }
    }

    fn user_filter_active(&self) -> bool {
        self.filter.is_empty() && !self.user_filter.is_empty()
    }

    pub fn user_filter_text(&self) -> &str {
        &self.user_filter
    }

    fn set_tab(&mut self, tab: Tab) -> Action {
        self.tab = tab;
        Action::Redraw
    }

    fn toggle_sort(&mut self, key: SortKey) -> Action {
        if self.sort == key {
            self.sort_desc = !self.sort_desc;
        } else {
            self.sort = key;
            self.sort_desc = true;
        }
        self.selected = 0;
        Action::Redraw
    }

    pub fn clamp_selection(&mut self, len: usize) {
        if self.selected >= len {
            self.selected = len.saturating_sub(1);
        }
    }
}

/// Thin wrapper so `Frame` based callers can reuse the buffer renderer.
pub fn draw(frame: &mut Frame, app: &App, snap: &Snapshot, hist: &History, theme: &Theme) {
    let area = frame.area();
    render(area, frame.buffer_mut(), app, snap, hist, theme);
}

/// Top level render: menu bar, body, footer.
pub fn render(area: Rect, buf: &mut Buffer, app: &App, snap: &Snapshot, hist: &History, theme: &Theme) {
    // btop fills its own background unless the user asked for transparency.
    if let Some(bg) = theme.background() {
        buf.set_style(area, Style::default().bg(bg));
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(6),
            Constraint::Length(1),
        ])
        .split(area);
    draw_menu(chunks[0], buf, app, snap, theme);
    draw_body(chunks[1], buf, app, snap, hist, theme);
    draw_footer(chunks[2], buf, app, snap, theme);
    if app.help {
        draw_help(area, buf, theme);
    }
    if app.kill_pending {
        draw_kill_prompt(area, buf, app, theme);
    }
}

/// btop-style top bar: tab chips on the left, clock and status on the right.
fn draw_menu(area: Rect, buf: &mut Buffer, app: &App, snap: &Snapshot, _theme: &Theme) {
    let mut x = area.left();
    for (index, tab) in Tab::ALL.iter().enumerate() {
        let label = format!(" {} {} ", index + 1, tab.label());
        let active = *tab == app.tab;
        let style = if active {
            Style::default()
                .fg(ratatui::style::Color::Black)
                .bg(crate::theme::HI_FG)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(crate::theme::MAIN_FG)
        };
        for (offset, ch) in label.chars().enumerate() {
            if let Some(cell) = buf.cell_mut((x + offset as u16, area.top())) {
                cell.set_symbol(ch.to_string().as_str());
                cell.set_style(style);
            }
        }
        x += label.chars().count() as u16;
    }
    // Memory sub-views are exposed here while the MEM tab is open. They are
    // labelled, not numbered: digits always switch tabs, `m` cycles the view.
    if app.tab == Tab::Mem {
        for view in MemView::ALL.iter() {
            let label = format!(" {} ", view.label());
            let active = *view == app.mem_view;
            let style = if active {
                Style::default()
                    .fg(ratatui::style::Color::Black)
                    .bg(crate::theme::VRAM_ACCENT)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(crate::theme::DIM)
            };
            for (offset, ch) in label.chars().enumerate() {
                if let Some(cell) = buf.cell_mut((x + offset as u16, area.top())) {
                    cell.set_symbol(ch.to_string().as_str());
                    cell.set_style(style);
                }
            }
            x += label.chars().count() as u16;
        }
    }

    // Clock, pinned mid-bar like btop.
    let clock = crate::collect::system::clock_string();
    if !clock.is_empty() && area.width > 60 {
        let clock_x = area.left() + (area.width / 2).saturating_sub(clock.len() as u16 / 2);
        if clock_x > x {
            for (offset, ch) in clock.chars().enumerate() {
                if let Some(cell) = buf.cell_mut((clock_x + offset as u16, area.top())) {
                    cell.set_symbol(ch.to_string().as_str());
                    cell.set_style(Style::default().fg(crate::theme::STRONG));
                }
            }
        }
    }

    // Right side: battery, sampling interval, hostname.
    let mut right = Vec::new();
    if app.sim {
        right.push(Span::styled(
            " SIMULATED ",
            Style::default()
                .fg(ratatui::style::Color::Black)
                .bg(crate::theme::VRAM_ACCENT)
                .add_modifier(Modifier::BOLD),
        ));
        right.push(Span::raw(" "));
    }
    if let Some(battery) = &snap.battery {
        if let Some(capacity) = battery.capacity {
            right.push(Span::styled(
                format!("{} {:.0}% ", battery.name, capacity),
                Style::default()
                    .fg(if capacity < 15.0 {
                        crate::theme::BAD
                    } else {
                        crate::theme::GOOD
                    })
                    .add_modifier(Modifier::BOLD),
            ));
        }
    }
    right.push(Span::styled(
        format!("{}ms ", app.interval_ms),
        Style::default().fg(crate::theme::DIM),
    ));
    if !snap.hostname.is_empty() {
        right.push(Span::styled(
            snap.hostname.clone(),
            Style::default().fg(crate::theme::CPU_ACCENT),
        ));
    }
    let width: usize = right.iter().map(|span| span.content.chars().count()).sum();
    if (width as u16) < area.width.saturating_sub(8) {
        buf.set_line(
            area.right().saturating_sub(width as u16),
            area.top(),
            &Line::from(right),
            width as u16,
        );
    }
}

/// `K` confirmation overlay.
fn draw_kill_prompt(area: Rect, buf: &mut Buffer, app: &App, theme: &Theme) {
    let text = match app.kill_target {
        Some(pid) => format!(" send SIGTERM to PID {}?  [y] yes   any other key cancels ", pid),
        None => " no process selected ".to_string(),
    };
    let width = (text.chars().count() + 2) as u16;
    let popup = center(area, width.min(area.width), 3.min(area.height));
    Clear.render(popup, buf);
    theme.block("kill", crate::theme::BAD).render(popup, buf);
    buf.set_line(
        popup.x + 1,
        popup.y + 1,
        &Line::from(Span::styled(text, Style::default().fg(crate::theme::WARN))),
        popup.width.saturating_sub(2),
    );
}

fn draw_body(area: Rect, buf: &mut Buffer, app: &App, snap: &Snapshot, hist: &History, theme: &Theme) {
    match app.tab {
        Tab::Dash => draw_dash(area, buf, app, snap, hist, theme),
        Tab::Cpu => cpu::full(area, buf, app, snap, hist, theme),
        Tab::Gpu => gpu::full(area, buf, app, snap, hist, theme),
        Tab::Mem => mem::full(area, buf, app, snap, hist, theme),
        Tab::Proc => proc::full(area, buf, app, snap, theme),
        Tab::Io => draw_io(area, buf, app, snap, hist, theme),
    }
}

/// Dashboard, laid out like btop:
///
/// ```text
/// ┌──────────────────────────────────────────────────────────────┐
/// │ cpu: per-CPU usages (left)  │  total CPU histogram (larger)   │
/// ├─────────────────────────────┴────────────────────────────────┤
/// │ gpu0: usage histogram (left) │  GPU / PWR / VRAM / GTT (right)│
/// ├──────────────────────────────────────────────────────────────┤
/// │ proc (left)                  │  mem, disks, net (right, stacked)
/// └──────────────────────────────────────────────────────────────┘
/// ```
fn draw_dash(area: Rect, buf: &mut Buffer, app: &App, snap: &Snapshot, hist: &History, theme: &Theme) {
    // The CPU box needs its top block plus as many rows as the core grid; the GPU
    // box fits its histogram, four bars and a sensor line.
    let cell_min: u16 = 18;
    // The cores panel is the 58% one (right of the histogram); size height off it.
    let core_box_width = (area.width as u32 * 58 / 100) as u16;
    let columns = ((core_box_width.saturating_sub(2)) / cell_min).max(1) as usize;
    let core_rows = snap.cpu.per_core.len().div_ceil(columns.max(1)) as u16;
    let head = cpu::head_rows(area.height);
    // borders (2) + top block + one row per core-grid row
    let cpu_height = (head + core_rows + 2).clamp(6, (area.height / 2).max(6));
    let gpu_height = 8u16.min(area.height / 4);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(cpu_height),
            Constraint::Length(gpu_height),
            Constraint::Min(6),
        ])
        .split(area);

    // CPU row mirrors the GPU row: usage histogram on the left, cores to its right.
    let cpu_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(rows[0]);
    cpu::histogram_box(cpu_cols[0], buf, snap, hist, theme, app.mirror_graphs);
    cpu::btop_box(cpu_cols[1], buf, app, snap, hist, theme);

    // GPU row: usage histogram on the left, readings on the right.
    let gpu_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(rows[1]);
    gpu::histogram_box(gpu_cols[0], buf, snap, hist, theme, app.mirror_graphs);
    gpu::btop_box(gpu_cols[1], buf, snap, hist, theme);

    let bottom = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(46), Constraint::Percentage(54)])
        .split(rows[2]);
    // Left half: memory above, disks and network side by side beneath it.
    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(bottom[0]);
    mem::btop_box(left[0], buf, snap, hist, theme);
    let io = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(left[1]);
    system::disk_box(io[0], buf, snap, hist, theme);
    system::net_box(io[1], buf, snap, hist, theme, app.mirror_graphs);
    // Right half: the process list, full height.
    proc::btop_box(bottom[1], buf, app, snap, theme);
}

/// Network and disk I/O tab (the dashboard keeps to the four main boxes).
fn draw_io(area: Rect, buf: &mut Buffer, app: &App, snap: &Snapshot, hist: &History, theme: &Theme) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);
    system::net_box(rows[0], buf, snap, hist, theme, app.mirror_graphs);
    system::disk_box(rows[1], buf, snap, hist, theme);
}

fn draw_footer(area: Rect, buf: &mut Buffer, app: &App, snap: &Snapshot, _theme: &Theme) {
    let left = if app.filter_active {
        let (prefix, text) = if app.user_filter_text().is_empty() {
            ("filter", app.filter.clone())
        } else {
            ("user", app.user_filter_text().to_string())
        };
        format!(" {}> {}#", prefix, text)
    } else {
        format!(
            " q quit · ? help · Tab tab · m mem-view · f filter · {}ms · {}",
            app.interval_ms,
            if app.paused { "paused" } else { "live" }
        )
    };
    let right = format!(
        "procs {} · gpu {:.1}s/{:.0}ms · sort {}{} ",
        snap.procs.len(),
        if snap.gpu_scan_age.is_finite() {
            snap.gpu_scan_age
        } else {
            0.0
        },
        snap.gpu_scan_ms,
        if app.sort_desc { "-" } else { "+" },
        app.sort.label()
    );
    buf.set_line(
        area.left(),
        area.top(),
        &Line::from(Span::styled(left, Style::default().fg(crate::theme::TEXT))),
        area.width,
    );
    let right_len = right.chars().count() as u16;
    if right_len < area.width {
        buf.set_line(
            area.right().saturating_sub(right_len),
            area.top(),
            &Line::from(Span::styled(right, Style::default().fg(crate::theme::DIM))),
            right_len,
        );
    }
}

fn draw_help(area: Rect, buf: &mut Buffer, theme: &Theme) {
    let lines: Vec<&str> = vec![
        "amdtop keys",
        "",
        "  q / Ctrl-C   quit                 ? / h    toggle this help",
        "  Tab / 1-6    switch tab           space    pause sampling",
        "  m            cycle memory view    +/-      sampling interval",
        "",
        "  process list",
        "    up/down j/k select             PgUp/PgDn page",
        "    f          filter command       /        filter by user",
        "    c %CPU   p PID   n %MEM   t TIME+   u USER   N COMMAND",
        "    g GTT    v VRAM  r GPU total       G      GPU-only rows",
        "    K then y   SIGTERM the selected process",
        "",
        "  display",
        "    M          toggle mirrored (centred) usage histograms",
        "    e          process tree on/off",
        "",
        "  Memory views: CPU RAM = what the kernel gives processes.",
        "  GPU = VRAM carveout + GTT (system RAM pinned for the GPU).",
        "  TTM = kernel pool internals (needs sudo). Unified = how the",
        "  single LPDDR5X pool is split between CPU and GPU.",
        "",
        "  press any key to close",
    ];
    let height = (lines.len() + 2) as u16;
    let width = lines
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(40) as u16
        + 4;
    let popup = center(
        area,
        width.min(area.width.saturating_sub(2)),
        height.min(area.height.saturating_sub(2)),
    );
    Clear.render(popup, buf);
    let block = theme.block("help", crate::theme::CPU_BOX);
    block.render(popup, buf);
    let inner = popup.inner(Margin::new(1, 1));
    let paragraph = Paragraph::new(
        lines
            .iter()
            .map(|line| Line::from(*line))
            .collect::<Vec<Line>>(),
    );
    paragraph.render(inner, buf);
}

fn center(area: Rect, width: u16, height: u16) -> Rect {
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::Backend;
    use crate::sim::SimBackend;
    use ratatui::buffer::Buffer;

    fn buffer_text(buf: &Buffer) -> String {
        let area = buf.area;
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                if let Some(cell) = buf.cell((x, y)) {
                    out.push_str(cell.symbol());
                }
            }
            out.push('\n');
        }
        out
    }

    fn render_tab(tab: Tab, mem_view: MemView) -> String {
        let mut backend = SimBackend::new();
        let mut history = History::new(64, 32);
        let mut snap = backend.sample();
        for _ in 0..60 {
            snap = backend.sample();
            history.push(&snap);
        }
        let mut app = App::new(&crate::args::Args::default());
        app.tab = tab;
        app.mem_view = mem_view;
        app.sim = true;
        let area = Rect {
            x: 0,
            y: 0,
            width: 150,
            height: 44,
        };
        let mut buf = Buffer::empty(area);
        let theme = Theme::new(false, true);
        render(area, &mut buf, &app, &snap, &history, &theme);
        buffer_text(&buf)
    }

    #[test]
    fn dashboard_shows_cpu_gpu_mem_and_procs() {
        let text = render_tab(Tab::Dash, MemView::Unified);
        assert!(text.contains("CPU"), "cpu panel missing");
        assert!(text.contains("Radeon 8060S"), "gpu name missing");
        assert!(text.contains("MEM"), "mem panel missing");
        assert!(text.contains("PROC"), "proc panel missing");
        assert!(text.contains("SIMULATED"), "sim badge missing");
    }

    #[test]
    fn dashboard_shows_every_core_like_btop() {
        let text = render_tab(Tab::Dash, MemView::Unified);
        for core in ["C00", "C15", "C16", "C31"] {
            assert!(text.contains(core), "core row {} missing from the dashboard", core);
        }
        assert!(text.contains("16 cores"), "core count missing from the cpu box title");
    }

    #[test]
    fn dashboard_has_btop_boxes() {
        let text = render_tab(Tab::Dash, MemView::Unified);
        // btop preset: cpu and gpu0 on top, proc and mem underneath.
        for needle in ["cpu ·", "gpu0 ·", "proc ·", "mem ·"] {
            assert!(text.contains(needle), "missing box: {}", needle);
        }
        // menu bar furniture
        assert!(text.contains("strix-halo"), "hostname missing from the menu bar");
        assert!(text.contains("BAT0"), "battery missing from the menu bar");
        assert!(text.contains("1000ms"), "interval missing from the menu bar");
    }

    #[test]
    fn gtt_is_separated_from_cpu_memory() {
        let text = render_tab(Tab::Dash, MemView::Unified);
        // The dashboard must distinguish CPU RAM from GPU-pinned memory.
        assert!(text.contains("GTT"), "GTT row missing from memory panel");
        assert!(text.contains("CPU "), "CPU row missing from memory panel");
    }

    #[test]
    fn io_tab_shows_net_and_disks() {
        let text = render_tab(Tab::Io, MemView::Unified);
        assert!(text.contains("net ·"), "net box missing from the IO tab");
        assert!(text.contains("disks ·"), "disk box missing from the IO tab");
    }

    #[test]
    fn every_memory_view_renders_numbers() {
        for view in MemView::ALL {
            let text = render_tab(Tab::Mem, view);
            assert!(
                text.contains("GiB") || text.contains("MiB"),
                "view {:?} rendered no sizes",
                view
            );
            for label in ["Dash", "CPU", "GPU", "MEM", "PROC"] {
                assert!(text.contains(label), "tab bar missing {}", label);
            }
        }
    }

    #[test]
    fn ttm_view_explains_missing_root_access() {
        let mut backend = SimBackend::new();
        let mut snap = backend.sample();
        snap.ttm.available = false;
        snap.ttm.error = Some("/sys/kernel/debug is not readable".to_string());
        snap.ttm.path = None;
        let history = History::new(8, 32);
        let mut app = App::new(&crate::args::Args::default());
        app.tab = Tab::Mem;
        app.mem_view = MemView::Ttm;
        let area = Rect {
            x: 0,
            y: 0,
            width: 140,
            height: 40,
        };
        let mut buf = Buffer::empty(area);
        render(area, &mut buf, &app, &snap, &history, &Theme::new(false, false));
        let text = buffer_text(&buf);
        assert!(text.contains("sudo amdtop"), "no root hint shown");
        assert!(text.contains("not readable"), "no error text shown");
    }

    #[test]
    fn process_tab_shows_gpu_columns() {
        let text = render_tab(Tab::Proc, MemView::Unified);
        assert!(text.contains("PID"), "no PID header");
        assert!(text.contains("GTT"), "no GTT column");
        assert!(text.contains("VRAM"), "no VRAM column");
        assert!(text.contains("llama-server"), "process rows missing");
        assert!(text.contains("TIME+"), "no TIME+ column");
    }

    #[test]
    fn cpu_tab_shows_cores_and_sensors() {
        let text = render_tab(Tab::Cpu, MemView::Unified);
        assert!(text.contains("C00"), "per-core rows missing");
        assert!(text.contains("Tctl"), "CPU temperature missing");
        assert!(text.contains("physical"), "topology line missing");
    }

    #[test]
    fn gpu_tab_shows_engines_and_memory() {
        let text = render_tab(Tab::Gpu, MemView::Unified);
        assert!(text.contains("VRAM"), "VRAM gauge missing");
        assert!(text.contains("GTT"), "GTT gauge missing");
        assert!(text.contains("compute"), "engine list missing");
        assert!(text.contains("2.90 GHz"), "clock range missing");
    }

    #[test]
    fn global_heat_cycle_switches_memory_view() {
        let mut app = App::new(&crate::args::Args::default());
        app.mem_view = MemView::Cpu;
        for expected in [MemView::Gpu, MemView::Ttm, MemView::Unified, MemView::Cpu] {
            let key = KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE);
            app.on_key(&key);
            assert_eq!(app.mem_view, expected);
            assert_eq!(app.tab, Tab::Mem);
        }
    }

    #[test]
    fn filter_and_sort_keys_work() {
        let mut app = App::new(&crate::args::Args::default());
        app.on_key(&KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
        assert_eq!(app.sort, SortKey::Gtt);
        app.on_key(&KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
        assert!(!app.sort_desc);
        app.on_key(&KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
        assert!(app.filter_active);
        for ch in "llama".chars() {
            app.on_key(&KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        assert_eq!(app.filter, "llama");
        app.on_key(&KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(!app.filter_active);
    }

    #[test]
    fn filter_matches_only_matching_processes() {
        let mut backend = SimBackend::new();
        let snap = backend.sample();
        let mut app = App::new(&crate::args::Args::default());
        app.filter = "llama".to_string();
        let rows = proc::visible(&app, &snap);
        assert!(!rows.is_empty());
        assert!(rows.iter().all(|(p, _)| p.comm.contains("llama")));
        app.gpu_only = true;
        let rows = proc::visible(&app, &snap);
        assert!(rows.iter().all(|(p, _)| p.has_gpu()));
    }

    #[test]
    fn ctrl_c_quits() {
        let mut app = App::new(&crate::args::Args::default());
        let action = app.on_key(&KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(action, Action::Quit);
        assert!(app.quit);
    }
}
