//! amdtop entry point: argument handling, terminal setup and the main loop.

mod args;
mod collect;
mod json;
mod sim;
mod theme;
mod ui;
mod util;

use std::io::{self, Write};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;

use crate::collect::linux::LinuxBackend;
use crate::collect::{Backend, Sampler};
use crate::sim::SimBackend;
use crate::theme::Theme;

fn main() {
    if let Err(error) = run() {
        eprintln!("amdtop: {}", error);
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = args::from_env();
    if let Some(error) = &args.error {
        eprintln!("error: {}\n\n{}", error, args::HELP);
        std::process::exit(2);
    }
    if args.help {
        println!("{}", args::HELP);
        return Ok(());
    }
    if args.version {
        println!("amdtop {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let unicode = !args.ascii && unicode_supported();
    let mut theme = Theme::new(!unicode, !args.no_color);
    // The Linux console font has box-drawing and blocks but no braille: keep the
    // block meters and only fall back to ASCII for the graphs.
    if console_lacks_braille() && !args.ascii {
        theme = theme.with_graph_ascii(true);
    }
    if args.transparent {
        theme = theme.transparent();
    }

    if args.demo {
        let backend = SimBackend::new();
        return dispatch(backend, args, theme);
    }
    let backend = LinuxBackend::new();
    dispatch(backend, args, theme)
}

fn dispatch<B: Backend>(backend: B, args: args::Args, theme: Theme) -> Result<(), Box<dyn std::error::Error>> {
    if args.debug {
        return debug(backend, &theme);
    }
    if args.json {
        return json_mode(backend, args, theme);
    }
    if let Some(frames) = args.frames {
        return frames_mode(backend, args, theme, frames);
    }
    interactive(backend, args, theme)
}

/// `--debug`: print what was discovered.
fn debug<B: Backend>(mut backend: B, theme: &Theme) -> Result<(), Box<dyn std::error::Error>> {
    // Two samples spaced past the kernel jiffy, otherwise the /proc/stat delta
    // is zero and the CPU line always reads 0.0% on real hardware.
    let _first = backend.sample();
    std::thread::sleep(Duration::from_millis(250));
    let snapshot = backend.sample();
    println!("amdtop {} — discovery", env!("CARGO_PKG_VERSION"));
    println!(
        "  {:<14} meters={}, graphs={} (TERM={}, LANG={})",
        "glyphs",
        if theme.ascii { "ascii" } else { "blocks" },
        if theme.graph_ascii { "ascii" } else { "braille" },
        std::env::var("TERM").unwrap_or_default(),
        std::env::var("LANG").unwrap_or_default()
    );
    for (label, value) in backend.info() {
        println!("  {:<14} {}", label, value);
    }
    println!("\n  snapshot");
    println!("  {:<14} {:.1}% total, {} cores", "cpu", snapshot.cpu.total * 100.0, snapshot.cpu.per_core.len());
    match &snapshot.gpu {
        Some(gpu) => {
            println!(
                "  {:<14} busy {}  vram {}/{}  gtt {}/{}",
                "gpu",
                util::human_pct(gpu.busy),
                util::human_bytes(gpu.vram_used.unwrap_or(0)),
                util::human_bytes(gpu.vram_total.unwrap_or(0)),
                util::human_bytes(gpu.gtt_used.unwrap_or(0)),
                util::human_bytes(gpu.gtt_total.unwrap_or(0))
            );
            println!(
                "  {:<14} clock {}  power {}  temp {}",
                "gpu sensors",
                util::human_freq(gpu.sclk),
                gpu.power_w.map(|p| format!("{:.1} W", p)).unwrap_or_else(|| "-".to_string()),
                gpu.hottest()
                    .map(|(label, value)| format!("{} {:.0}°C", label, value))
                    .unwrap_or_else(|| "-".to_string())
            );
        }
        None => println!("  {:<14} none", "gpu"),
    }
    match &snapshot.npu {
        Some(npu) => {
            println!(
                "  {:<14} {}  {}x{} cols  busy {}  power {}",
                "npu",
                npu.label(),
                npu.cols.unwrap_or(0),
                npu.rows.unwrap_or(0),
                util::human_pct(npu.busy),
                npu.power_w
                    .map(|power| format!("{:.1} W", power))
                    .unwrap_or_else(|| "-".to_string())
            );
            println!(
                "  {:<14} tops {}/{}  clock {}  fw {}  {}",
                "npu array",
                npu.tops_curr.unwrap_or(0),
                npu.tops_max.unwrap_or(0),
                util::human_freq(npu.npu_mhz),
                npu.fw,
                npu.bdf
            );
            println!(
                "  {:<14} {} contexts, {} active{}",
                "npu contexts",
                npu.contexts.len(),
                npu.contexts.iter().filter(|context| context.active()).count(),
                npu.error
                    .as_ref()
                    .map(|error| format!("  ({})", error))
                    .unwrap_or_default()
            );
        }
        None => println!("  {:<14} none", "npu"),
    }
    println!(
        "  {:<14} used {} / {} (gtt {} excluded: {})",
        "memory",
        util::human_bytes(snapshot.mem.used),
        util::human_bytes(snapshot.mem.total),
        util::human_bytes(snapshot.gpu.as_ref().and_then(|g| g.gtt_used).unwrap_or(0)),
        util::human_bytes(
            snapshot
                .mem
                .used
                .saturating_sub(snapshot.gpu.as_ref().and_then(|g| g.gtt_used).unwrap_or(0))
        )
    );
    println!(
        "  {:<14} {} ({})",
        "ttm",
        if snapshot.ttm.available { "readable" } else { "unavailable" },
        snapshot.ttm.error.clone().unwrap_or_else(|| "-".to_string())
    );
    let gpu_procs: Vec<_> = snapshot.procs.iter().filter(|proc| proc.has_gpu()).collect();
    println!("  {:<14} {} processes, {} with GPU memory", "processes", snapshot.procs.len(), gpu_procs.len());
    for proc in gpu_procs.iter().take(10) {
        println!(
            "      {:>7} {:<18} gtt {:>9}  vram {:>9}  clients {}",
            proc.pid,
            util::truncate(&proc.comm, 18),
            util::human_bytes(proc.gtt),
            util::human_bytes(proc.vram),
            proc.gpu_clients
        );
    }
    Ok(())
}

/// `--json`: take two samples (rates need a delta) and print the snapshot.
fn json_mode<B: Backend>(backend: B, args: args::Args, _theme: Theme) -> Result<(), Box<dyn std::error::Error>> {
    let mut sampler = Sampler::new(backend, 16);
    std::thread::sleep(Duration::from_millis(args.interval_ms.min(2000)));
    sampler.tick();
    print!("{}", json::snapshot(&sampler.last, 100));
    io::stdout().flush()?;
    Ok(())
}

/// `--frames`: render without a terminal, for screenshots and CI.
fn frames_mode<B: Backend>(
    backend: B,
    args: args::Args,
    theme: Theme,
    frames: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = args.size;
    let area = Rect {
        x: 0,
        y: 0,
        width,
        height,
    };
    let mut sampler = Sampler::new(backend, args.history);
    let mut app = ui::App::new(&args);
    app.sim = sampler.backend.is_sim();
    let mut output = String::new();
    // Fill the graphs before capturing so the output looks like a live session.
    // The ticks must be spaced by more than the kernel's jiffy (10 ms) or the
    // /proc/stat deltas are zero and every headless graph reads as idle.
    let gap = std::time::Duration::from_millis(args.interval_ms.min(50).max(20));
    let warmup = (args.history as u64).min(60);
    for _ in 0..warmup {
        std::thread::sleep(gap);
        sampler.tick();
    }
    for frame_index in 0..frames {
        std::thread::sleep(gap);
        sampler.tick();
        let mut buffer = ratatui::buffer::Buffer::empty(area);
        ui::render(area, &mut buffer, &app, &sampler.last, &sampler.history, &theme);
        if frames > 1 {
            output.push_str(&format!("=== frame {} ===\n", frame_index + 1));
        }
        if args.plain {
            output.push_str(&buffer_to_text(&buffer));
        } else {
            output.push_str(&buffer_to_ansi(&buffer));
        }
        output.push('\n');
    }
    let mut sink: Box<dyn Write> = if args.out == "-" {
        Box::new(io::stdout())
    } else {
        Box::new(std::fs::File::create(&args.out)?)
    };
    sink.write_all(output.as_bytes())?;
    sink.flush()?;
    Ok(())
}

fn buffer_to_text(buffer: &ratatui::buffer::Buffer) -> String {
    let area = buffer.area;
    let mut out = String::new();
    for y in 0..area.height {
        let mut line = String::new();
        for x in 0..area.width {
            if let Some(cell) = buffer.cell((x, y)) {
                line.push_str(cell.symbol());
            }
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

/// Approximate ANSI rendering: one escape per cell whose colour changed.
fn buffer_to_ansi(buffer: &ratatui::buffer::Buffer) -> String {
    use ratatui::style::{Color, Modifier};
    let area = buffer.area;
    let fg_code = |color: Color| -> String {
        match color {
            Color::Reset => "\x1b[39m".to_string(),
            Color::Black => "\x1b[30m".to_string(),
            Color::Red => "\x1b[31m".to_string(),
            Color::Green => "\x1b[32m".to_string(),
            Color::Yellow => "\x1b[33m".to_string(),
            Color::Blue => "\x1b[34m".to_string(),
            Color::Magenta => "\x1b[35m".to_string(),
            Color::Cyan => "\x1b[36m".to_string(),
            Color::Gray => "\x1b[37m".to_string(),
            Color::DarkGray => "\x1b[90m".to_string(),
            Color::LightRed => "\x1b[91m".to_string(),
            Color::LightGreen => "\x1b[92m".to_string(),
            Color::LightYellow => "\x1b[93m".to_string(),
            Color::LightBlue => "\x1b[94m".to_string(),
            Color::LightMagenta => "\x1b[95m".to_string(),
            Color::LightCyan => "\x1b[96m".to_string(),
            Color::White => "\x1b[97m".to_string(),
            Color::Indexed(index) => format!("\x1b[38;5;{}m", index),
            Color::Rgb(r, g, b) => format!("\x1b[38;2;{};{};{}m", r, g, b),
        }
    };
    let bg_code = |color: Color| -> String {
        match color {
            Color::Reset => "\x1b[49m".to_string(),
            Color::Black => "\x1b[40m".to_string(),
            Color::Red => "\x1b[41m".to_string(),
            Color::Green => "\x1b[42m".to_string(),
            Color::Yellow => "\x1b[43m".to_string(),
            Color::Blue => "\x1b[44m".to_string(),
            Color::Magenta => "\x1b[45m".to_string(),
            Color::Cyan => "\x1b[46m".to_string(),
            Color::Gray => "\x1b[47m".to_string(),
            Color::DarkGray => "\x1b[100m".to_string(),
            Color::LightRed => "\x1b[101m".to_string(),
            Color::LightGreen => "\x1b[102m".to_string(),
            Color::LightYellow => "\x1b[103m".to_string(),
            Color::LightBlue => "\x1b[104m".to_string(),
            Color::LightMagenta => "\x1b[105m".to_string(),
            Color::LightCyan => "\x1b[106m".to_string(),
            Color::White => "\x1b[107m".to_string(),
            Color::Indexed(index) => format!("\x1b[48;5;{}m", index),
            Color::Rgb(r, g, b) => format!("\x1b[48;2;{};{};{}m", r, g, b),
        }
    };
    let mut out = String::new();
    for y in 0..area.height {
        let mut line = String::new();
        let mut current: Option<(Color, Color, bool)> = None;
        for x in 0..area.width {
            let Some(cell) = buffer.cell((x, y)) else {
                continue;
            };
            let bold = cell.modifier.contains(Modifier::BOLD);
            let pair = (cell.fg, cell.bg, bold);
            if current != Some(pair) {
                line.push_str(&fg_code(cell.fg));
                line.push_str(&bg_code(cell.bg));
                line.push_str(if bold { "\x1b[1m" } else { "\x1b[22m" });
                current = Some(pair);
            }
            line.push_str(cell.symbol());
        }
        line.push_str("\x1b[0m");
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// Interactive TUI.
fn interactive<B: Backend>(
    backend: B,
    args: args::Args,
    theme: Theme,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut sampler = Sampler::new(backend, args.history);
    let mut app = ui::App::new(&args);
    app.sim = sampler.backend.is_sim();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, crossterm::cursor::Hide)?;
    let terminal_backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(terminal_backend)?;
    let _guard = TerminalGuard;

    let mut last_tick = Instant::now();
    loop {
        terminal.draw(|frame| {
            ui::draw(frame, &app, &sampler.last, &sampler.history, &theme);
        })?;

        let interval = Duration::from_millis(app.interval_ms);
        // While paused there is no tick to wait for: poll the full interval so
        // the loop sleeps on input instead of spinning at 100% CPU.
        let timeout = if app.paused {
            interval
        } else {
            interval.saturating_sub(last_tick.elapsed())
        };
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    handle_key(&mut app, &mut sampler, key)?;
                }
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
        if app.quit {
            break;
        }
        if !app.paused && last_tick.elapsed() >= interval {
            sampler.tick();
            last_tick = Instant::now();
        }
    }
    Ok(())
}

fn handle_key(
    app: &mut ui::App,
    sampler: &mut Sampler<impl Backend>,
    key: KeyEvent,
) -> Result<(), Box<dyn std::error::Error>> {
    if app.kill_pending {
        if let (KeyCode::Char('y'), Some(pid)) = (key.code, app.kill_target) {
            let result = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
            app.message = Some(if result == 0 {
                format!("sent SIGTERM to {}", pid)
            } else {
                format!("could not signal {} ({})", pid, io::Error::last_os_error())
            });
        }
        app.kill_pending = false;
        app.kill_target = None;
        return Ok(());
    }
    match app.on_key(&key) {
        ui::Action::Quit => {
            app.quit = true;
        }
        ui::Action::SetInterval(_) | ui::Action::Redraw => {
            if app.kill_pending && app.kill_target.is_none() {
                app.kill_target = ui::proc::selected_pid(app, &sampler.last);
            }
        }
        ui::Action::None => {}
    }
    if app.kill_pending && app.kill_target.is_none() {
        app.kill_target = ui::proc::selected_pid(app, &sampler.last);
    }
    let visible = ui::proc::visible(app, &sampler.last).len();
    app.clamp_selection(visible);
    Ok(())
}

/// Restores the terminal even when the app unwinds.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, crossterm::cursor::Show);
    }
}

/// The Linux text console (`TERM=linux`) has no braille in its font, so graphs
/// come out as stray characters even when `LANG` says UTF-8. Only the graphs are
/// affected: its fonts do have the block and box-drawing glyphs.
fn console_lacks_braille() -> bool {
    match std::env::var("TERM") {
        Ok(term) => term.to_ascii_lowercase().starts_with("linux"),
        Err(_) => false,
    }
}

/// Whether the locale supports UTF-8 (block glyphs, box drawing).
fn unicode_supported() -> bool {
    for key in ["LC_ALL", "LC_CTYPE", "LANG"] {
        if let Ok(value) = std::env::var(key) {
            if !value.is_empty() {
                return value.to_ascii_lowercase().contains("utf");
            }
        }
    }
    true
}
