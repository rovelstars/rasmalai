use diagnostics::theme::{AuraColor, AuraTheme};

pub const BUS_WIDTH: usize = 68;

pub fn fmt_duration_ms(theme: &AuraTheme, ms: f64) -> String {
    let num = format!("{:>6.2}", ms);
    format!(
        "{}{}",
        theme.paint(AuraColor::Orange, &num),
        theme.paint(AuraColor::Muted, " ms")
    )
}

pub fn fmt_bytes(theme: &AuraTheme, bytes: u64) -> String {
    let text = plain_bytes(bytes);
    match text.find(' ') {
        Some(i) => format!(
            "{}{}",
            theme.paint(AuraColor::Orange, &text[..i]),
            theme.paint(AuraColor::Muted, &text[i..])
        ),
        None => text,
    }
}

fn plain_bytes(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        format!("{:.2} MB", bytes as f64 / 1_048_576.0)
    } else {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    }
}

fn plain_bytes_len(bytes: u64) -> usize {
    plain_bytes(bytes).chars().count()
}

pub fn peak_mb() -> Option<f64> {
    let text = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("VmHWM:") {
            let kb: f64 = rest.split_whitespace().next()?.parse().ok()?;
            return Some(kb / 1024.0);
        }
    }
    None
}

fn rule(theme: &AuraTheme, prefix: &str, title: &str) -> String {
    let dashes =
        BUS_WIDTH.saturating_sub(prefix.chars().count() + title.chars().count());
    format!(
        "{}{}{}",
        theme.paint(AuraColor::Muted, prefix),
        theme.paint(AuraColor::Text, title),
        theme.paint(AuraColor::Muted, &"─".repeat(dashes))
    )
}

pub struct BusStep {
    pub name: String,
    pub detail: String,
    pub duration_ms: f64,
}

pub fn render_bus(
    theme: &AuraTheme,
    title: &str,
    steps: &[BusStep],
    artifact: &str,
    size_bytes: u64,
    total_ms: f64,
    peak_mb: Option<f64>,
) -> String {
    let mut out = String::new();
    out.push_str(&rule(theme, "┌─ ", title));
    out.push('\n');
    out.push_str(&theme.paint(AuraColor::Muted, "│"));
    out.push('\n');
    let detail_width = BUS_WIDTH - 3 - 1 - 1 - 1 - 10 - 1 - 1 - 9;
    for (i, step) in steps.iter().enumerate() {
        let last = i + 1 == steps.len();
        let branch = if last { "└──" } else { "├──" };
        let node = if last {
            theme.paint(AuraColor::Green, "●")
        } else {
            theme.paint(AuraColor::Cyan, "○")
        };
        let mut detail = step.detail.clone();
        if detail.chars().count() > detail_width {
            detail = detail.chars().take(detail_width).collect();
        }
        while detail.chars().count() < detail_width {
            detail.push(' ');
        }
        out.push_str(&format!(
            "{} {} {:<10} {detail} {}",
            theme.paint(AuraColor::Muted, branch),
            node,
            step.name,
            fmt_duration_ms(theme, step.duration_ms)
        ));
        out.push('\n');
    }
    out.push_str(&theme.paint(AuraColor::Muted, "│"));
    out.push('\n');
    let size = fmt_bytes(theme, size_bytes);
    let badge = format!(
        "{} {} [{}]",
        theme.paint(AuraColor::Muted, "└─ artifact:"),
        theme.paint(AuraColor::Green, artifact),
        size
    );
    let badge_plain_len = ("└─ artifact: ".chars().count())
        + artifact.chars().count()
        + 3
        + plain_bytes_len(size_bytes);
    let dashes = BUS_WIDTH.saturating_sub(badge_plain_len + 1);
    out.push_str(&badge);
    out.push(' ');
    out.push_str(&theme.paint(AuraColor::Muted, &"─".repeat(dashes)));
    out.push('\n');
    let total_num = format!("{:>6.2}", total_ms);
    let total_styled = format!(
        "total: {}{}",
        theme.paint(AuraColor::Orange, &total_num),
        theme.paint(AuraColor::Muted, " ms")
    );
    let total_plain = "total: ".len() + total_num.len() + " ms".len();
    out.push_str(&pad_left(&total_styled, total_plain));
    out.push('\n');
    if let Some(peak) = peak_mb {
        let peak_text = format!("{peak:.2} MB");
        let peak_styled = format!("peak:     {}", theme.paint(AuraColor::Text, &peak_text));
        let peak_plain = "peak:     ".len() + peak_text.chars().count();
        out.push_str(&pad_left(&peak_styled, peak_plain));
        out.push('\n');
    }
    out
}

fn pad_left(styled: &str, plain: usize) -> String {
    let pad = BUS_WIDTH.saturating_sub(plain);
    format!("{}{}", " ".repeat(pad), styled)
}

pub fn colorize_test_line(theme: &AuraTheme, line: &str) -> String {
    if let Some(rest) = line.strip_prefix("  ✓") {
        return format!(
            "  {}{}",
            theme.paint(AuraColor::Green, "✓"),
            paint_duration_tail(theme, rest)
        );
    }
    if let Some(rest) = line.strip_prefix("  ✗") {
        return format!(
            "  {}{}",
            theme.paint(AuraColor::Red, "✗"),
            paint_duration_tail(theme, rest)
        );
    }
    if line.starts_with("┌─ failure") || line.starts_with("└─ note") {
        let mut chars = line.chars();
        let mut head = String::new();
        for _ in 0..3 {
            if let Some(c) = chars.next() {
                head.push(c);
            }
        }
        let rest: String = chars.collect();
        return format!("{}{}", theme.paint(AuraColor::Muted, &head), rest);
    }
    if !line.starts_with(' ') && !line.is_empty() && line.contains("::") && !line.contains(' ') {
        return theme.paint(AuraColor::Cyan, line);
    }
    line.to_string()
}

fn paint_duration_tail(theme: &AuraTheme, rest: &str) -> String {
    let (body, unit) = match rest.strip_suffix(" ms") {
        Some(body) => (body, " ms"),
        None => return rest.to_string(),
    };
    let num_len = body
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .count();
    let split = body.len() - num_len;
    format!(
        "{}{}{}",
        &body[..split],
        theme.paint(AuraColor::Orange, &body[split..]),
        theme.paint(AuraColor::Muted, unit)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_lock_right_boundary() {
        let theme = AuraTheme::plain();
        let a = fmt_duration_ms(&theme, 1.8);
        let b = fmt_duration_ms(&theme, 15.97);
        assert_eq!(a, "  1.80 ms");
        assert_eq!(b, " 15.97 ms");
        assert_eq!(a.len(), b.len());
    }

    #[test]
    fn bytes_format() {
        let theme = AuraTheme::plain();
        assert_eq!(fmt_bytes(&theme, 364_544), "356 KB");
        assert_eq!(fmt_bytes(&theme, 5_033_165), "4.80 MB");
    }

    #[test]
    fn bus_structure_plain() {
        let theme = AuraTheme::plain();
        let steps = vec![
            BusStep {
                name: "parse".to_string(),
                detail: "lex & parse sources".to_string(),
                duration_ms: 1.8,
            },
            BusStep {
                name: "link".to_string(),
                detail: "executable link".to_string(),
                duration_ms: 1.2,
            },
        ];
        let out = render_bus(
            &theme,
            "build: miner [release] (x86_64-linux)",
            &steps,
            "bin/miner",
            364_544,
            3.0,
            Some(4.8),
        );
        assert!(out.contains("┌─ build: miner [release] (x86_64-linux)"), "{out}");
        assert!(out.contains("├── ○ parse"), "{out}");
        assert!(out.contains("└── ● link"), "{out}");
        assert!(out.contains("artifact: bin/miner [356 KB]"), "{out}");
        assert!(out.contains("total:"), "{out}");
        assert!(out.contains("peak:     4.80 MB"), "{out}");
        for line in out.lines() {
            if line.chars().count() <= 1 {
                continue;
            }
            assert_eq!(line.chars().count(), BUS_WIDTH, "ragged: {line:?}");
        }
    }

    #[test]
    fn bus_colors_marks() {
        let theme = AuraTheme {
            enabled: true,
            truecolor: true,
        };
        assert!(fmt_duration_ms(&theme, 1.8).contains("\x1b[38;2;255;202;133m  1.80\x1b[0m"));
        assert!(fmt_duration_ms(&theme, 1.8).contains("\x1b[38;2;109;109;109m ms\x1b[0m"));
    }

    #[test]
    fn test_lines_colorize_without_breaking_names() {
        let theme = AuraTheme {
            enabled: true,
            truecolor: true,
        };
        let line = "  ✓ matrix4x4_mul   0.20 ms";
        let out = colorize_test_line(&theme, line);
        assert!(out.contains("\x1b[38;2;97;255;202m✓\x1b[0m"), "{out}");
        assert!(out.contains("matrix4x4_mul"), "{out}");
        let plain = colorize_test_line(&AuraTheme::plain(), line);
        assert_eq!(plain, line);
    }
}
