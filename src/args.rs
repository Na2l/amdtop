//! Command line parsing (hand rolled: no extra dependencies).

use std::env;

#[derive(Debug, Clone)]
pub struct Args {
    pub demo: bool,
    pub interval_ms: u64,
    pub history: usize,
    pub frames: Option<u64>,
    pub out: String,
    pub plain: bool,
    pub json: bool,
    pub debug: bool,
    pub ascii: bool,
    pub no_color: bool,
    pub transparent: bool,
    pub tab: Option<String>,
    pub mem_view: Option<String>,
    pub gpu_only: bool,
    /// Fixed size for `--frames` rendering.
    pub size: (u16, u16),
    pub help: bool,
    pub version: bool,
    pub error: Option<String>,
}

impl Default for Args {
    fn default() -> Self {
        Args {
            demo: false,
            interval_ms: 1000,
            history: 600,
            frames: None,
            out: "-".to_string(),
            plain: false,
            json: false,
            debug: false,
            ascii: false,
            no_color: false,
            transparent: false,
            tab: None,
            mem_view: None,
            gpu_only: false,
            size: (120, 38),
            help: false,
            version: false,
            error: None,
        }
    }
}

pub const HELP: &str = "\
amdtop - btop-style monitor for AMD Strix Halo (CPU + GPU + VRAM/GTT/TTM + processes)

USAGE:
    amdtop [OPTIONS]

OPTIONS:
    -d, --demo              run against a simulated Strix Halo (no AMD GPU needed)
    -i, --interval <ms>     refresh interval in milliseconds [default: 1000]
        --history <n>       samples kept for graphs [default: 600]
        --tab <name>        initial tab: dash|cpu|gpu|mem|proc|io
        --mem-view <name>   initial memory view: cpu|gpu|ttm|unified
        --gpu-only          hide processes without GPU memory in the process list
        --size <WxH>        render size for --frames [default: 120x38]
        --ascii             disable unicode glyphs (ASCII meters and graphs)
        --no-color          monochrome output
        --transparent       do not paint btop's black background
    -h, --help              print this help
    -V, --version           print version

SCRIPTING / SCREENSHOTS:
        --json              print one JSON snapshot (top 100 procs) and exit
        --frames <n>        render n frames without a terminal (for CI/screenshots)
        --out <path>        output file for --frames ('-' = stdout)
        --plain             strip colours from --frames output
        --debug             print device discovery details and exit

KEYS:
    q / Ctrl-C  quit        ? / h     help
    Tab / 1-6   tabs        m         cycle memory views (CPU/GPU/TTM/Unified)
    +/-         interval    space     pause
    up/down j/k select      PgUp/PgDn page
    f           filter      /         filter by user
    c p n t u N g v r       sort by cpu/pid/mem/time/user/command/gtt/vram/gpu
    K then y    SIGTERM     e         process tree on/off
    M           mirrored graphs       G  GPU-only rows
";

fn needs_value(name: &str) -> bool {
    matches!(
        name,
        "--interval" | "-i" | "--history" | "--tab" | "--mem-view" | "--frames" | "--out" | "--size"
    )
}

pub fn parse<I: Iterator<Item = String>>(args: &mut I) -> Args {
    let mut out = Args::default();
    while let Some(arg) = args.next() {
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) => (name.to_string(), Some(value.to_string())),
            None => (arg.clone(), None),
        };
        if needs_value(&name) {
            let value = match inline.or_else(|| args.next()) {
                Some(value) => value,
                None => {
                    out.error = Some(format!("{} needs a value", name));
                    return out;
                }
            };
            match name.as_str() {
                "--interval" | "-i" => match value.parse::<u64>() {
                    Ok(value) => out.interval_ms = value.clamp(100, 60_000),
                    Err(_) => {
                        out.error = Some(format!("--interval needs a number of ms"));
                        return out;
                    }
                },
                "--history" => match value.parse::<usize>() {
                    Ok(value) => out.history = value.clamp(32, 100_000),
                    Err(_) => {
                        out.error = Some(format!("--history needs a number"));
                        return out;
                    }
                },
                "--frames" => match value.parse::<u64>() {
                    Ok(value) => out.frames = Some(value.max(1)),
                    Err(_) => {
                        out.error = Some(format!("--frames needs a number"));
                        return out;
                    }
                },
                "--size" => match parse_size(&value) {
                    Some(size) => out.size = size,
                    None => {
                        out.error = Some("--size expects WxH, e.g. 120x38".to_string());
                        return out;
                    }
                },
                "--tab" => out.tab = Some(value.to_ascii_lowercase()),
                "--mem-view" => out.mem_view = Some(value.to_ascii_lowercase()),
                "--out" => out.out = value,
                _ => {}
            }
            continue;
        }
        match name.as_str() {
            "-d" | "--demo" => out.demo = true,
            "--json" => out.json = true,
            "--plain" => out.plain = true,
            "--debug" => out.debug = true,
            "--ascii" => out.ascii = true,
            "--no-color" => out.no_color = true,
            "--transparent" => out.transparent = true,
            "--gpu-only" => out.gpu_only = true,
            "-h" | "--help" => out.help = true,
            "-V" | "--version" => out.version = true,
            other => {
                out.error = Some(format!("unknown argument: {}", other));
                return out;
            }
        }
    }
    out
}

fn parse_size(text: &str) -> Option<(u16, u16)> {
    let (width, height) = text.split_once(['x', 'X'])?;
    let width: u16 = width.trim().parse().ok()?;
    let height: u16 = height.trim().parse().ok()?;
    if width < 40 || height < 10 {
        return None;
    }
    Some((width, height))
}

pub fn from_env() -> Args {
    parse(&mut env::args().skip(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_str(text: &str) -> Args {
        parse(&mut text.split_whitespace().map(str::to_string))
    }

    #[test]
    fn defaults() {
        let args = parse_str("");
        assert_eq!(args.interval_ms, 1000);
        assert!(!args.demo);
        assert!(args.error.is_none());
    }

    #[test]
    fn flags_and_values() {
        let args = parse_str("--demo -i 500 --tab=proc --ascii");
        assert!(args.demo);
        assert_eq!(args.interval_ms, 500);
        assert_eq!(args.tab.as_deref(), Some("proc"));
        assert!(args.ascii);
    }

    #[test]
    fn transparent_flag() {
        assert!(parse_str("--transparent").transparent);
        assert!(!parse_str("").transparent);
    }

    #[test]
    fn parses_size() {
        let args = parse_str("--size 160x48");
        assert_eq!(args.size, (160, 48));
        assert!(parse_str("--size nope").error.is_some());
        assert!(parse_str("--size 10x10").error.is_some());
    }

    #[test]
    fn rejects_bad_values() {
        assert!(parse_str("--interval fast").error.is_some());
        assert!(parse_str("--nope").error.is_some());
        assert!(parse_str("--tab").error.is_some());
    }
}
