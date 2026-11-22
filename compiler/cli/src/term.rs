//! Terminal layout and live progress for the CLI.
//!
//! The rule this module enforces: a pipe gets stable lines with no cursor
//! control, a real terminal gets one redrawn block. Every caller keeps its
//! permanent output identical in both modes, so logs and test assertions do
//! not change shape just because a TTY happens to be attached.

use diagnostics::theme::{AuraColor, AuraTheme, ColorMode};
use std::io::{IsTerminal, Write};
use std::time::{Duration, Instant};

/// Pick the color mode from raw arguments before clap parses them, so help
/// text and every later line use the same policy.
pub fn detect_color_mode(raw: &[String]) -> ColorMode {
    let mut mode = ColorMode::Auto;
    let mut i = 0;
    while i < raw.len() {
        let arg = &raw[i];
        if arg == "--no-color" {
            mode = ColorMode::Never;
        } else if let Some(value) = arg.strip_prefix("--color=") {
            mode = color_mode_of(value).unwrap_or(mode);
        } else if arg == "--color" {
            if let Some(value) = raw.get(i + 1) {
                mode = color_mode_of(value).unwrap_or(mode);
                i += 1;
            }
        }
        i += 1;
    }
    mode
}

fn color_mode_of(value: &str) -> Option<ColorMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(ColorMode::Auto),
        "always" | "yes" | "true" => Some(ColorMode::Always),
        "never" | "no" | "false" => Some(ColorMode::Never),
        _ => None,
    }
}

/// Columns used when the terminal size cannot be read. Kept at the classic
/// 80 so output stays readable in default terminals and CI logs.
const DEFAULT_WIDTH: usize = 80;

pub fn width() -> usize {
    if let Some(columns) = std::env::var("COLUMNS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|n| (20..=400).contains(n))
    {
        return columns;
    }
    tty_width().unwrap_or(DEFAULT_WIDTH).clamp(20, 400)
}

#[cfg(unix)]
fn tty_width() -> Option<usize> {
    use std::os::fd::AsRawFd;
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    let fd = std::io::stderr().as_raw_fd();
    if unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) } == 0 && ws.ws_col > 0 {
        Some(ws.ws_col as usize)
    } else {
        None
    }
}

#[cfg(not(unix))]
fn tty_width() -> Option<usize> {
    None
}

/// True when the process can safely move the cursor and redraw.
pub fn live_supported() -> bool {
    if std::env::var_os("RNX_NO_PROGRESS").is_some_and(|v| !v.is_empty()) {
        return false;
    }
    if std::env::var("TERM").map(|t| t == "dumb").unwrap_or(false) {
        return false;
    }
    std::io::stderr().is_terminal()
}

pub fn ascii_only() -> bool {
    std::env::var("TERM").map(|t| t == "dumb").unwrap_or(false)
        || std::env::var_os("RNX_ASCII").is_some()
}

pub fn spinner(frame: usize) -> &'static str {
    if ascii_only() {
        const FRAMES: [&str; 4] = ["-", "\\", "|", "/"];
        return FRAMES[frame % FRAMES.len()];
    }
    const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    FRAMES[frame % FRAMES.len()]
}

pub fn human_duration_ms(ms: f64) -> String {
    if ms >= 60_000.0 {
        let minutes = (ms / 60_000.0).floor();
        let seconds = (ms - minutes * 60_000.0) / 1000.0;
        format!("{minutes:.0}m {seconds:04.1}s")
    } else if ms >= 1_000.0 {
        format!("{:.2} s", ms / 1000.0)
    } else if ms >= 10.0 {
        format!("{ms:.0} ms")
    } else {
        format!("{ms:.1} ms")
    }
}

pub fn human_count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Print width of a string that may contain SGR escapes.
pub fn visible_width(text: &str) -> usize {
    let mut count = 0;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            continue;
        }
        count += 1;
    }
    count
}

/// Truncate to `max` visible columns, closing any SGR state that was open.
pub fn truncate(text: &str, max: usize) -> String {
    if visible_width(text) <= max {
        return text.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let limit = max.saturating_sub(1);
    let mut out = String::new();
    let mut visible = 0;
    let mut saw_escape = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            saw_escape = true;
            out.push(c);
            if chars.peek() == Some(&'[') {
                out.push(chars.next().unwrap());
                for c in chars.by_ref() {
                    out.push(c);
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            continue;
        }
        if visible == limit {
            break;
        }
        out.push(c);
        visible += 1;
    }
    out.push('…');
    if saw_escape {
        out.push_str("\x1b[0m");
    }
    out
}

/// Remove SGR and cursor escape sequences so a line can be classified.
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Pending,
    Running(usize),
    Done,
    Failed,
}

/// One progress row: a state glyph, a short label, and a right-aligned detail.
pub fn row(theme: &AuraTheme, mark: Mark, label: &str, detail: &str, columns: usize) -> String {
    let (glyph, label_color) = match mark {
        Mark::Pending => (theme.paint(AuraColor::Muted, "·"), AuraColor::Muted),
        Mark::Running(frame) => (
            theme.paint(AuraColor::Cyan, spinner(frame)),
            AuraColor::Text,
        ),
        Mark::Done => (theme.paint(AuraColor::Green, "✓"), AuraColor::Text),
        Mark::Failed => (theme.paint(AuraColor::Red, "✗"), AuraColor::Text),
    };
    let head = format!("  {glyph} {}", theme.paint(label_color, label));
    if detail.is_empty() {
        return truncate(&head, columns);
    }
    let detail = theme.paint(AuraColor::Muted, detail);
    let gap = columns.saturating_sub(visible_width(&head) + visible_width(&detail));
    if gap >= 2 {
        format!("{head}{}{detail}", " ".repeat(gap))
    } else {
        truncate(&format!("{head}  {detail}"), columns)
    }
}

pub struct Live {
    out: Box<dyn Write + Send>,
    enabled: bool,
    width: usize,
    lines: Vec<String>,
    drawn: usize,
    last: Instant,
    interval: Duration,
}

impl Live {
    pub fn new(quiet: bool) -> Live {
        Live::with_writer(quiet, Box::new(std::io::stderr()))
    }

    pub fn with_writer(quiet: bool, out: Box<dyn Write + Send>) -> Live {
        Live {
            out,
            enabled: !quiet && live_supported(),
            width: width(),
            lines: Vec::new(),
            drawn: 0,
            last: Instant::now(),
            interval: Duration::from_millis(80),
        }
    }

    /// Test and embedding hook: force a block on with a fixed width.
    pub fn forced(width: usize, out: Box<dyn Write + Send>) -> Live {
        Live {
            out,
            enabled: true,
            width,
            lines: Vec::new(),
            drawn: 0,
            last: Instant::now(),
            interval: Duration::ZERO,
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn set(&mut self, lines: Vec<String>) {
        if !self.enabled {
            return;
        }
        self.lines = lines;
        self.draw(false);
    }

    /// Clear the block so a permanent line can be written above it.
    pub fn suspend(&mut self) {
        self.clear();
    }

    /// Redraw the block after a permanent line.
    pub fn resume(&mut self) {
        self.draw(true);
    }

    pub fn finish(&mut self) {
        if !self.enabled {
            return;
        }
        self.clear();
        let _ = self.out.flush();
    }

    fn clear(&mut self) {
        if !self.enabled || self.drawn == 0 {
            return;
        }
        let mut buf = String::new();
        buf.push_str(&format!("\r\x1b[{}A", self.drawn));
        buf.push_str("\x1b[J");
        let _ = self.out.write_all(buf.as_bytes());
        let _ = self.out.flush();
        self.drawn = 0;
    }

    fn draw(&mut self, force: bool) {
        if !self.enabled {
            return;
        }
        if !force && self.last.elapsed() < self.interval {
            return;
        }
        if self.lines.is_empty() {
            return;
        }
        let mut buf = String::new();
        if self.drawn > 0 {
            buf.push_str(&format!("\r\x1b[{}A", self.drawn));
        }
        buf.push_str("\r\x1b[J");
        for line in &self.lines {
            buf.push_str(&truncate(line, self.width));
            buf.push('\n');
        }
        self.drawn = self.lines.len();
        self.last = Instant::now();
        let _ = self.out.write_all(buf.as_bytes());
        let _ = self.out.flush();
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        self.finish();
    }
}

/// Drives a redraw from a background thread while the main thread blocks.
/// Stopping joins the ticker, so no frame can land after the caller prints.
pub struct Ticker {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Ticker {
    pub fn start(mut frame: impl FnMut(usize) + Send + 'static) -> Ticker {
        use std::sync::atomic::{AtomicBool, Ordering};
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let handle = std::thread::spawn(move || {
            // Give the first phase (dependency resolution, cache probes) a
            // moment to emit any warnings before the block takes the line.
            let mut waited = 0u64;
            while waited < 120 {
                if flag.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
                waited += 10;
            }
            let mut n = 0usize;
            while !flag.load(Ordering::Relaxed) {
                frame(n);
                n = n.wrapping_add(1);
                std::thread::sleep(Duration::from_millis(80));
            }
        });
        Ticker {
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for Ticker {
    fn drop(&mut self) {
        use std::sync::atomic::Ordering;
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct Sink(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl Write for Sink {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn sink() -> (Sink, std::sync::Arc<std::sync::Mutex<Vec<u8>>>) {
        let bytes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        (Sink(bytes.clone()), bytes)
    }

    fn text(bytes: &std::sync::Arc<std::sync::Mutex<Vec<u8>>>) -> String {
        String::from_utf8(bytes.lock().unwrap().clone()).unwrap()
    }

    #[test]
    fn ansi_escapes_do_not_count_as_columns() {
        assert_eq!(visible_width("\x1b[38;2;1;2;3mabc\x1b[0m"), 3);
        assert_eq!(visible_width("plain"), 5);
    }

    #[test]
    fn truncation_keeps_escapes_balanced() {
        let cut = truncate("\x1b[38;5;122mabcdef\x1b[0m", 4);
        assert_eq!(visible_width(&cut), 4);
        assert!(cut.ends_with("\x1b[0m"), "{cut:?}");
    }

    #[test]
    fn rows_right_align_detail() {
        let theme = AuraTheme::plain();
        let line = row(&theme, Mark::Done, "parse", "12 ms", 40);
        assert_eq!(visible_width(&line), 40);
        assert!(line.starts_with("  ✓ parse"), "{line:?}");
        assert!(line.ends_with("12 ms"), "{line:?}");
    }

    #[test]
    fn live_block_redraws_in_place() {
        let (out, bytes) = sink();
        let mut live = Live::forced(30, Box::new(out));
        live.set(vec![row(&AuraTheme::plain(), Mark::Running(0), "build", "1.2 s", 30)]);
        live.set(vec![row(&AuraTheme::plain(), Mark::Done, "build", "1.4 s", 30)]);
        live.finish();
        let out = text(&bytes);
        assert!(out.contains("\x1b[1A"), "{out:?}");
        assert!(out.contains("\x1b[J"), "{out:?}");
        assert!(out.contains("build"), "{out:?}");
        assert!(!out.contains("\n\n"), "{out:?}");
    }

    #[test]
    fn plain_live_writes_only_said_lines() {
        let (out, bytes) = sink();
        let mut live = Live {
            out: Box::new(out),
            enabled: false,
            width: 30,
            lines: Vec::new(),
            drawn: 0,
            last: Instant::now(),
            interval: Duration::ZERO,
        };
        live.set(vec!["hidden".to_string()]);
        live.finish();
        assert_eq!(text(&bytes), "");
    }

    #[test]
    fn duration_has_one_shape_per_magnitude() {
        assert_eq!(human_duration_ms(0.4), "0.4 ms");
        assert_eq!(human_duration_ms(12.4), "12 ms");
        assert_eq!(human_duration_ms(1240.0), "1.24 s");
        assert_eq!(human_duration_ms(64_000.0), "1m 04.0s");
    }

    #[test]
    fn counts_group_thousands() {
        assert_eq!(human_count(7), "7");
        assert_eq!(human_count(1234), "1,234");
        assert_eq!(human_count(1_234_567), "1,234,567");
    }

    #[test]
    fn color_flag_beats_default() {
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(detect_color_mode(&args(&[])), ColorMode::Auto);
        assert_eq!(
            detect_color_mode(&args(&["--color", "always"])),
            ColorMode::Always
        );
        assert_eq!(
            detect_color_mode(&args(&["--color=never"])),
            ColorMode::Never
        );
        assert_eq!(detect_color_mode(&args(&["--no-color"])), ColorMode::Never);
    }
}
