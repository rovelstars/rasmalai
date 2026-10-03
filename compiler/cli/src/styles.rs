use clap::builder::styling::{AnsiColor, Color, Style, Styles};

pub fn aura_clap_styles() -> Styles {
    Styles::styled()
        .header(Style::new().bold().fg_color(Some(Color::Ansi(AnsiColor::BrightMagenta))))
        .literal(Style::new().fg_color(Some(Color::Ansi(AnsiColor::Cyan))))
        .usage(Style::new().bold().fg_color(Some(Color::Ansi(AnsiColor::BrightMagenta))))
        .placeholder(Style::new().fg_color(Some(Color::Ansi(AnsiColor::Magenta))))
        .error(Style::new().bold().fg_color(Some(Color::Ansi(AnsiColor::Red))))
        .valid(Style::new().fg_color(Some(Color::Ansi(AnsiColor::BrightGreen))))
        .invalid(Style::new().fg_color(Some(Color::Ansi(AnsiColor::Red))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styles_build_without_panic() {
        let _ = aura_clap_styles();
    }
}
