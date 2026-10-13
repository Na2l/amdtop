//! Small helpers shared by the collectors and the UI: safe file reads and
//! human readable number formatting.

use std::fs;
use std::path::Path;

/// Read a whole file to a String, `None` on any error (missing, EACCES, ...).
pub fn read_string(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path.as_ref())
        .map(|s| s.replace('\0', ""))
        .ok()
}

/// Read a file without touching NUL bytes (needed for `/proc/<pid>/cmdline`).
pub fn read_raw(path: impl AsRef<Path>) -> Option<String> {
    std::fs::read(path.as_ref())
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).to_string())
}

/// Read a sysfs style file and trim whitespace.
pub fn read_trim(path: impl AsRef<Path>) -> Option<String> {
    read_string(path).map(|s| s.trim().to_string())
}

pub fn read_u64(path: impl AsRef<Path>) -> Option<u64> {
    read_trim(path)?.parse().ok()
}

pub fn clamp(value: f64, low: f64, high: f64) -> f64 {
    if value < low {
        low
    } else if value > high {
        high
    } else {
        value
    }
}

/// `used / total` as a 0..1 ratio, `None` when the total is unknown/zero.
pub fn ratio(used: Option<u64>, total: Option<u64>) -> Option<f64> {
    match (used, total) {
        (Some(u), Some(t)) if t > 0 => Some(clamp(u as f64 / t as f64, 0.0, 1.0)),
        _ => None,
    }
}

/// Split bytes into `(value, unit)` at the natural magnitude.
pub fn split_bytes(nbytes: u64) -> (f64, &'static str) {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut value = nbytes as f64;
    let mut idx = 0usize;
    while value >= 1024.0 && idx < UNITS.len() - 1 {
        value /= 1024.0;
        idx += 1;
    }
    (value, UNITS[idx])
}

/// `1.5 GiB`, right aligned to `width` when non zero.
pub fn human_bytes(nbytes: u64) -> String {
    let (value, unit) = split_bytes(nbytes);
    if unit == "B" {
        format!("{} {}", value as u64, unit)
    } else if value >= 100.0 {
        format!("{:.0} {}", value, unit)
    } else {
        format!("{:.1} {}", value, unit)
    }
}

/// Compact form for table columns: `1.5G`, `512M`, `0`.
pub fn human_bytes_compact(nbytes: u64) -> String {
    let (value, unit) = split_bytes(nbytes);
    let unit = unit.replace("iB", "").replace('B', "");
    if unit.is_empty() {
        format!("{}", value as u64)
    } else if value >= 100.0 {
        format!("{:.0}{}", value, unit)
    } else {
        format!("{:.1}{}", value, unit)
    }
}

/// `1.2 MiB/s`, `0 B/s`.
pub fn human_rate(bytes_per_sec: Option<f64>) -> String {
    match bytes_per_sec {
        Some(value) => format!("{}/s", human_bytes(value.max(0.0) as u64)),
        None => "-".to_string(),
    }
}

/// MHz -> `800 MHz` / `2.40 GHz`.
pub fn human_freq(mhz: Option<f64>) -> String {
    match mhz {
        Some(v) if v >= 1000.0 => format!("{:.2} GHz", v / 1000.0),
        Some(v) => format!("{} MHz", v.round() as i64),
        None => "-".to_string(),
    }
}

/// `0.42` -> ` 42.0%`.
pub fn human_pct(ratio: Option<f64>) -> String {
    match ratio {
        Some(v) => format!("{:5.1}%", v * 100.0),
        None => "    -".to_string(),
    }
}

/// Uptime style duration: `3d 04:12`, `1:02:03`, `04:12`.
pub fn human_seconds(seconds: f64) -> String {
    let total = seconds.max(0.0) as u64;
    let days = total / 86400;
    let hours = (total % 86400) / 3600;
    let minutes = (total % 3600) / 60;
    let secs = total % 60;
    if days > 0 {
        format!("{}d {:02}:{:02}", days, hours, minutes)
    } else if hours > 0 {
        format!("{}:{:02}:{:02}", hours, minutes, secs)
    } else {
        format!("{:02}:{:02}", minutes, secs)
    }
}

/// Cumulative process CPU time in `top` `TIME+` style.
pub fn human_cputime(seconds: f64) -> String {
    let centis = (seconds * 100.0).round() as u64;
    let hundredths = centis % 100;
    let total_seconds = centis / 100;
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let secs = total_seconds % 60;
    if hours > 0 {
        format!("{}:{:02}:{:02}.{:02}", hours, minutes, secs, hundredths)
    } else {
        format!("{:02}:{:02}.{:02}", minutes, secs, hundredths)
    }
}

/// Parse `"1234 KiB"`, `"8MiB"`, `"4096"` into bytes.
pub fn parse_size_with_unit(text: &str) -> Option<u64> {
    let text = text.trim();
    let split_at = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(text.len());
    let (num, unit) = text.split_at(split_at);
    if num.is_empty() {
        return None;
    }
    let value: f64 = num.parse().ok()?;
    let unit = unit.trim().to_ascii_uppercase();
    let factor: u64 = match unit.as_str() {
        "" | "B" => 1,
        "K" | "KB" | "KIB" => 1024,
        "M" | "MB" | "MIB" => 1024u64.pow(2),
        "G" | "GB" | "GIB" => 1024u64.pow(3),
        "T" | "TB" | "TIB" => 1024u64.pow(4),
        "P" | "PB" | "PIB" => 1024u64.pow(5),
        _ => return None,
    };
    Some((value * factor as f64) as u64)
}

/// Truncate to `width` display columns, appending `…` when cut.
pub fn truncate(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count <= width {
        return text.to_string();
    }
    if width <= 1 {
        return "…".to_string();
    }
    let mut out: String = text.chars().take(width - 1).collect();
    out.push('…');
    out
}

/// Pad/truncate to exactly `width` columns (left aligned).
pub fn fit(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let count = text.chars().count();
    if count >= width {
        return truncate(text, width);
    }
    let mut out = text.to_string();
    out.push_str(&" ".repeat(width - count));
    out
}

/// Pad to exactly `width` columns (right aligned).
pub fn fit_right(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let count = text.chars().count();
    if count >= width {
        return truncate(text, width);
    }
    let mut out = " ".repeat(width - count);
    out.push_str(text);
    out
}

/// Parse `/proc` style `key: 1234 kB` lines into `(key, value)` pairs.
pub fn proc_key_values(text: &str) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        let token = rest
            .split_whitespace()
            .next()
            .unwrap_or("0")
            .replace(',', "");
        if let Ok(value) = token.parse::<u64>() {
            out.push((key.trim().to_string(), value));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_bytes() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(1024), "1.0 KiB");
        assert_eq!(human_bytes(1536), "1.5 KiB");
        assert_eq!(human_bytes(3 * 1024 * 1024 * 1024), "3.0 GiB");
        assert_eq!(human_bytes_compact(1536 * 1024 * 1024), "1.5G");
        assert_eq!(human_bytes_compact(0), "0");
    }

    #[test]
    fn formats_rates() {
        assert_eq!(human_rate(Some(1024.0)), "1.0 KiB/s");
        assert_eq!(human_rate(Some(0.0)), "0 B/s");
        assert_eq!(human_rate(None), "-");
    }

    #[test]
    fn parses_units() {
        assert_eq!(parse_size_with_unit("1234 KiB"), Some(1234 * 1024));
        assert_eq!(parse_size_with_unit("2MiB"), Some(2 * 1024 * 1024));
        assert_eq!(parse_size_with_unit("4096"), Some(4096));
        assert_eq!(parse_size_with_unit("bogus"), None);
    }

    #[test]
    fn durations() {
        assert_eq!(human_seconds(75.0), "01:15");
        assert_eq!(human_seconds(3725.0), "1:02:05");
        assert_eq!(human_seconds(90000.0), "1d 01:00");
        assert_eq!(human_cputime(65.34), "01:05.34");
    }

    #[test]
    fn truncation() {
        assert_eq!(truncate("hello world", 8), "hello w…");
        assert_eq!(fit("ab", 5), "ab   ");
        assert_eq!(fit_right("ab", 5), "   ab");
    }
}
