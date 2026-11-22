use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

static FORCE_PLAIN: AtomicBool = AtomicBool::new(false);

pub fn force_plain_output() {
    FORCE_PLAIN.store(true, Ordering::Relaxed);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorMode {
    Auto,
    Always,
    Never,
}

static COLOR_MODE: AtomicU8 = AtomicU8::new(0);

pub fn set_color_mode(mode: ColorMode) {
    let code = match mode {
        ColorMode::Auto => 0,
        ColorMode::Always => 1,
        ColorMode::Never => 2,
    };
    COLOR_MODE.store(code, Ordering::Relaxed);
}

pub fn color_mode() -> ColorMode {
    match COLOR_MODE.load(Ordering::Relaxed) {
        1 => ColorMode::Always,
        2 => ColorMode::Never,
        _ => ColorMode::Auto,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

fn env_truthy(name: &str) -> bool {
    std::env::var(name)
        .map(|v| {
            let v = v.trim();
            !v.is_empty() && v != "0"
        })
        .unwrap_or(false)
}

fn env_off(name: &str) -> bool {
    std::env::var(name).map(|v| v.trim() == "0").unwrap_or(false)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuraColor {
    Purple,
    Green,
    Orange,
    Pink,
    Cyan,
    Red,
    Text,
    Muted,
}

impl AuraColor {
    pub fn rgb(self) -> (u8, u8, u8) {
        match self {
            AuraColor::Purple => (162, 119, 255),
            AuraColor::Green => (97, 255, 202),
            AuraColor::Orange => (255, 202, 133),
            AuraColor::Pink => (246, 148, 255),
            AuraColor::Cyan => (130, 226, 255),
            AuraColor::Red => (255, 103, 103),
            AuraColor::Text => (237, 236, 238),
            AuraColor::Muted => (109, 109, 109),
        }
    }

    pub fn ansi256(self) -> u8 {
        match self {
            AuraColor::Purple => 141,
            AuraColor::Green => 122,
            AuraColor::Orange => 216,
            AuraColor::Pink => 213,
            AuraColor::Cyan => 117,
            AuraColor::Red => 203,
            AuraColor::Text => 255,
            AuraColor::Muted => 242,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AuraTheme {
    pub enabled: bool,
    pub truecolor: bool,
}

impl AuraTheme {
    pub fn plain() -> AuraTheme {
        AuraTheme {
            enabled: false,
            truecolor: false,
        }
    }

    pub fn active() -> AuraTheme {
        AuraTheme::for_stream(Stream::Stdout)
    }

    pub fn for_stream(stream: Stream) -> AuraTheme {
        if FORCE_PLAIN.load(Ordering::Relaxed) {
            return AuraTheme::plain();
        }
        let enabled = match color_mode() {
            ColorMode::Never => false,
            // An explicit request wins over environment hints, which is what
            // makes `--color=always` usable when stdout is a pipe or a test.
            ColorMode::Always => true,
            ColorMode::Auto => {
                if std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()) {
                    false
                } else if std::env::var("TERM").map(|t| t == "dumb").unwrap_or(false) {
                    false
                } else if env_off("CLICOLOR") {
                    false
                } else if env_truthy("CLICOLOR_FORCE") || env_truthy("FORCE_COLOR") {
                    true
                } else {
                    match stream {
                        Stream::Stdout => std::io::stdout().is_terminal(),
                        Stream::Stderr => std::io::stderr().is_terminal(),
                    }
                }
            }
        };
        if !enabled {
            return AuraTheme::plain();
        }
        let colorterm = std::env::var("COLORTERM").unwrap_or_default();
        let truecolor = colorterm == "truecolor" || colorterm == "24bit";
        AuraTheme {
            enabled: true,
            truecolor,
        }
    }

    pub fn paint(&self, color: AuraColor, text: &str) -> String {
        if !self.enabled {
            return text.to_string();
        }
        if self.truecolor {
            let (r, g, b) = color.rgb();
            format!("\x1b[38;2;{r};{g};{b}m{text}\x1b[0m")
        } else {
            format!("\x1b[38;5;{}m{text}\x1b[0m", color.ansi256())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rich() -> AuraTheme {
        AuraTheme {
            enabled: true,
            truecolor: true,
        }
    }

    fn ansi() -> AuraTheme {
        AuraTheme {
            enabled: true,
            truecolor: false,
        }
    }

    #[test]
    fn palette_values_match_spec() {
        assert_eq!(AuraColor::Purple.rgb(), (162, 119, 255));
        assert_eq!(AuraColor::Purple.ansi256(), 141);
        assert_eq!(AuraColor::Green.rgb(), (97, 255, 202));
        assert_eq!(AuraColor::Green.ansi256(), 122);
        assert_eq!(AuraColor::Orange.rgb(), (255, 202, 133));
        assert_eq!(AuraColor::Orange.ansi256(), 216);
        assert_eq!(AuraColor::Pink.rgb(), (246, 148, 255));
        assert_eq!(AuraColor::Pink.ansi256(), 213);
        assert_eq!(AuraColor::Cyan.rgb(), (130, 226, 255));
        assert_eq!(AuraColor::Cyan.ansi256(), 117);
        assert_eq!(AuraColor::Red.rgb(), (255, 103, 103));
        assert_eq!(AuraColor::Red.ansi256(), 203);
        assert_eq!(AuraColor::Text.rgb(), (237, 236, 238));
        assert_eq!(AuraColor::Text.ansi256(), 255);
        assert_eq!(AuraColor::Muted.rgb(), (109, 109, 109));
        assert_eq!(AuraColor::Muted.ansi256(), 242);
    }

    #[test]
    fn paint_emits_truecolor_and_256_sequences() {
        assert_eq!(
            rich().paint(AuraColor::Red, "x"),
            "\x1b[38;2;255;103;103mx\x1b[0m"
        );
        assert_eq!(ansi().paint(AuraColor::Red, "x"), "\x1b[38;5;203mx\x1b[0m");
    }

    #[test]
    fn plain_theme_passes_text_through() {
        assert_eq!(AuraTheme::plain().paint(AuraColor::Red, "x"), "x");
    }

    #[test]
    fn explicit_color_mode_overrides_tty_detection() {
        let previous = color_mode();
        set_color_mode(ColorMode::Always);
        assert!(AuraTheme::for_stream(Stream::Stdout).enabled);
        set_color_mode(ColorMode::Never);
        assert!(!AuraTheme::for_stream(Stream::Stdout).enabled);
        set_color_mode(previous);
    }
}
