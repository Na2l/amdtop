//! btop's default colour palette, taken verbatim from `btop_theme.cpp`
//! (`Default_theme`), plus the 3-stop gradient interpolation btop uses.
//!
//! Two-digit values in btop's table (`"#ee"`) mean a repeated byte (`#eeeeee`).
//! The screen is painted with btop's `main_bg` unless `--transparent` is passed
//! (btop's `theme_background = False`), in which case the terminal's own
//! background shows through.

use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders};

//* Main *//
/// btop's `main_bg`: painted across the screen unless `--transparent`.
pub const MAIN_BG: Color = Color::Rgb(0x00, 0x00, 0x00);
pub const MAIN_FG: Color = Color::Rgb(0xcc, 0xcc, 0xcc);
pub const TITLE: Color = Color::Rgb(0xee, 0xee, 0xee);
pub const HI_FG: Color = Color::Rgb(0xb5, 0x40, 0x40);
pub const SELECTED_BG: Color = Color::Rgb(0x6a, 0x2f, 0x2f);
pub const SELECTED_FG: Color = Color::Rgb(0xee, 0xee, 0xee);
pub const INACTIVE_FG: Color = Color::Rgb(0x40, 0x40, 0x40);
pub const GRAPH_TEXT: Color = Color::Rgb(0x60, 0x60, 0x60);
pub const METER_BG: Color = Color::Rgb(0x40, 0x40, 0x40);
pub const PROC_MISC: Color = Color::Rgb(0x0d, 0xe7, 0x56);

//* Box accents *//
pub const CPU_BOX: Color = Color::Rgb(0x55, 0x6d, 0x59);
pub const MEM_BOX: Color = Color::Rgb(0x6c, 0x6c, 0x4b);
pub const NET_BOX: Color = Color::Rgb(0x5c, 0x58, 0x8d);
pub const PROC_BOX: Color = Color::Rgb(0x80, 0x52, 0x52);
/// btop has no GPU box; this is btop's own `proc_banner_bg` purple.
pub const GPU_BOX: Color = Color::Rgb(0x7b, 0x40, 0x7b);
/// Disks share the memory box colour in btop.
pub const DISK_BOX: Color = MEM_BOX;

//* Named accents used across the panels (all from the table above). *//
pub const TEXT: Color = MAIN_FG;
pub const STRONG: Color = TITLE;
pub const DIM: Color = INACTIVE_FG;
pub const ACCENT: Color = HI_FG;
pub const GOOD: Color = Color::Rgb(0xb5, 0xe6, 0x85);
pub const WARN: Color = Color::Rgb(0xff, 0xb8, 0x14);
pub const BAD: Color = Color::Rgb(0xff, 0x47, 0x69);
pub const GPU_ACCENT: Color = GPU_BOX;
pub const GTT_ACCENT: Color = Color::Rgb(0x74, 0xe6, 0xfc);
pub const VRAM_ACCENT: Color = Color::Rgb(0xff, 0xd7, 0x7a);
pub const CPU_ACCENT: Color = Color::Rgb(0x77, 0xca, 0x9b);
pub const IO_WRITE: Color = Color::Rgb(0x7d, 0x41, 0x80);

/// Which btop gradient a value should be painted with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gradient {
    /// CPU load: green -> yellow -> red.
    Cpu,
    /// Memory in use.
    Used,
    /// Memory available.
    Available,
    /// Page cache.
    Cached,
    /// Free memory.
    Free,
    /// Temperature: blue -> violet -> pink.
    Temp,
    /// Network receive.
    Download,
    /// Network transmit.
    Upload,
    /// Per-process CPU%.
    Process,
}

impl Gradient {
    /// The three btop stops: `(start, mid, end)`.
    const fn stops(self) -> ((u8, u8, u8), (u8, u8, u8), (u8, u8, u8)) {
        match self {
            Gradient::Cpu => ((0x77, 0xca, 0x9b), (0xcb, 0xc0, 0x6c), (0xdc, 0x4c, 0x4c)),
            Gradient::Used => ((0x59, 0x2b, 0x26), (0xd9, 0x62, 0x6d), (0xff, 0x47, 0x69)),
            Gradient::Available => ((0x4e, 0x3f, 0x0e), (0xff, 0xd7, 0x7a), (0xff, 0xb8, 0x14)),
            Gradient::Cached => ((0x16, 0x33, 0x50), (0x74, 0xe6, 0xfc), (0x26, 0xc5, 0xff)),
            Gradient::Free => ((0x38, 0x4f, 0x21), (0xb5, 0xe6, 0x85), (0xdc, 0xff, 0x85)),
            Gradient::Temp => ((0x48, 0x97, 0xd4), (0x54, 0x74, 0xe8), (0xff, 0x40, 0xb6)),
            Gradient::Download => ((0x29, 0x1f, 0x75), (0x4f, 0x43, 0xa3), (0xb0, 0xa9, 0xde)),
            Gradient::Upload => ((0x62, 0x06, 0x65), (0x7d, 0x41, 0x80), (0xdc, 0xaf, 0xde)),
            Gradient::Process => ((0x80, 0xd0, 0xa3), (0xdc, 0xd1, 0x79), (0xd4, 0x54, 0x54)),
        }
    }
}

fn lerp(a: u8, b: u8, t: f64) -> u8 {
    (a as f64 + (b as f64 - a as f64) * t).round().clamp(0.0, 255.0) as u8
}

/// Linear interpolation across btop's three stops, like btop's 101-step tables.
pub fn gradient_color(kind: Gradient, ratio: f64) -> Color {
    let ratio = crate::util::clamp(ratio, 0.0, 1.0);
    let (start, mid, end) = kind.stops();
    let (a, b, t) = if ratio < 0.5 {
        (start, mid, ratio * 2.0)
    } else {
        (mid, end, (ratio - 0.5) * 2.0)
    };
    Color::Rgb(lerp(a.0, b.0, t), lerp(a.1, b.1, t), lerp(a.2, b.2, t))
}

/// Glyph set with an ASCII fallback for non UTF-8 locales.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    /// Block glyphs replaced by ASCII (`--ascii`, non-UTF-8 locales).
    pub ascii: bool,
    /// Graphs only: the terminal font has no braille, so use an ASCII ramp while
    /// keeping the block meters, which such fonts usually do have.
    pub graph_ascii: bool,
    pub color: bool,
    /// Paint btop's `main_bg` over the terminal background (btop's
    /// `theme_background = True`), or let the terminal show through.
    pub paint_bg: bool,
}

impl Theme {
    pub fn new(ascii: bool, color: bool) -> Theme {
        Theme {
            ascii,
            graph_ascii: ascii,
            color,
            paint_bg: true,
        }
    }

    /// Draw graphs with ASCII glyphs while keeping block meters.
    pub fn with_graph_ascii(mut self, value: bool) -> Theme {
        self.graph_ascii = value;
        self
    }

    /// Leave the terminal's own background visible (btop's
    /// `theme_background = False`).
    pub fn transparent(mut self) -> Theme {
        self.paint_bg = false;
        self
    }

    /// The background to fill, or `None` to inherit the terminal's.
    pub fn background(&self) -> Option<Color> {
        if self.paint_bg && self.color {
            Some(MAIN_BG)
        } else {
            None
        }
    }

    /// Paint a value with a btop gradient (plain grey when colour is off).
    pub fn grad(&self, kind: Gradient, ratio: f64) -> Color {
        if !self.color {
            return Color::Reset;
        }
        gradient_color(kind, ratio)
    }

    pub fn braille_ok(&self) -> bool {
        !self.graph_ascii
    }

    pub fn full(&self) -> &'static str {
        if self.ascii {
            "#"
        } else {
            "█"
        }
    }

    /// Unfilled part of a meter: btop draws dashes in `meter_bg`.
    pub fn empty(&self) -> &'static str {
        if self.ascii {
            "-"
        } else {
            "─"
        }
    }

    pub fn eighth(&self, index: usize) -> &'static str {
        const BLOCKS: [&str; 9] = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
        const ASCII: [&str; 9] = [" ", ".", ":", "-", "=", "+", "*", "#", "@"];
        if self.graph_ascii {
            // A graded ramp keeps the graph readable on the Linux console font.
            ASCII[index.clamp(0, 8)]
        } else {
            BLOCKS[index.clamp(0, 8)]
        }
    }

    pub fn arrow(&self) -> &'static str {
        if self.ascii {
            ">"
        } else {
            "▸"
        }
    }

    pub fn meter_bg(&self) -> Color {
        if self.color {
            METER_BG
        } else {
            Color::Reset
        }
    }

    /// btop's `graph_text`: used for graph baselines and axis-ish text.
    pub fn graph_text(&self) -> Color {
        if self.color {
            GRAPH_TEXT
        } else {
            Color::Reset
        }
    }

    pub fn dim(&self) -> Color {
        if self.color {
            INACTIVE_FG
        } else {
            Color::Reset
        }
    }

    pub fn label(&self) -> Color {
        if self.color {
            MAIN_FG
        } else {
            Color::Reset
        }
    }

    pub fn strong(&self) -> Color {
        if self.color {
            TITLE
        } else {
            Color::Reset
        }
    }

    /// A btop-style bordered box in the given accent colour.
    pub fn block(&self, title: &str, accent: Color) -> Block<'_> {
        let border = if self.color { accent } else { MAIN_FG };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border));
        if title.is_empty() {
            return block;
        }
        block.title(title_span(title, border, self.color))
    }
}

/// btop renders box titles as a chip in the box colour with dark text.
fn title_span(title: &str, accent: Color, color: bool) -> ratatui::text::Span<'static> {
    let style = if color {
        Style::default()
            .fg(Color::Rgb(0x00, 0x00, 0x00))
            .bg(accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    ratatui::text::Span::styled(format!(" {} ", title), style)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gradient_endpoints_match_btop_stops() {
        assert_eq!(gradient_color(Gradient::Cpu, 0.0), Color::Rgb(0x77, 0xca, 0x9b));
        assert_eq!(gradient_color(Gradient::Cpu, 0.5), Color::Rgb(0xcb, 0xc0, 0x6c));
        assert_eq!(gradient_color(Gradient::Cpu, 1.0), Color::Rgb(0xdc, 0x4c, 0x4c));
    }

    #[test]
    fn gradient_interpolates_between_stops() {
        let quarter = gradient_color(Gradient::Used, 0.25);
        let three_quarters = gradient_color(Gradient::Used, 0.75);
        assert_ne!(quarter, three_quarters);
        assert_ne!(quarter, gradient_color(Gradient::Used, 0.0));
    }

    #[test]
    fn monochrome_theme_ignores_gradients() {
        let theme = Theme::new(false, false);
        assert_eq!(theme.grad(Gradient::Cpu, 0.9), Color::Reset);
        assert_eq!(theme.background(), None);
    }

    #[test]
    fn theme_paints_btop_background_unless_transparent() {
        let theme = Theme::new(false, true);
        assert_eq!(theme.background(), Some(MAIN_BG));
        assert_eq!(theme.transparent().background(), None);
    }
}
