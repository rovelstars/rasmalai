use super::theme::{AuraColor, AuraTheme};

pub struct TreeHook {
    pub title: String,
    pub location: Option<String>,
    pub context: Vec<(usize, String)>,
    pub pointer_line: usize,
    pub pointer_col: usize,
    pub got: Option<String>,
    pub expected: Option<String>,
    pub footer: Option<String>,
    pub footer2: Option<String>,
}

fn gutter_width(hook: &TreeHook) -> usize {
    hook.context
        .iter()
        .map(|(n, _)| n.to_string().len())
        .max()
        .unwrap_or(1)
        .max(2)
}

fn expand_tabs(line: &str) -> String {
    line.replace('\t', "    ")
}

pub fn render(theme: &AuraTheme, hook: &TreeHook) -> String {
    let mut out = String::new();
    let muted = |s: &str| theme.paint(AuraColor::Muted, s);
    out.push_str(&muted("┌─ "));
    out.push_str(&hook.title);
    out.push('\n');
    if let Some(location) = &hook.location {
        out.push_str(&muted("│  "));
        out.push_str(&theme.paint(AuraColor::Cyan, location));
        out.push('\n');
    }
    if hook.location.is_some() || !hook.context.is_empty() {
        out.push_str(&muted("│"));
        out.push('\n');
    }
    let width = gutter_width(hook);
    let blanks = " ".repeat(width);
    for (lineno, text) in &hook.context {
        let text = expand_tabs(text);
        out.push_str(&muted(&format!("│  {lineno:>width$} │  ")));
        out.push_str(&text);
        out.push('\n');
        if *lineno == hook.pointer_line {
            let pad = " ".repeat(hook.pointer_col);
            out.push_str(&muted(&format!("·  {blanks} │  {pad}")));
            out.push_str(&theme.paint(AuraColor::Red, "▲"));
            out.push('\n');
            if hook.got.is_some() || hook.expected.is_some() {
                out.push_str(&muted(&format!("·  {blanks} │  {pad}└──┬── ")));
                if let Some(got) = &hook.got {
                    out.push_str(&theme.paint(AuraColor::Pink, "got:"));
                    out.push_str("      ");
                    out.push_str(&theme.paint(AuraColor::Text, got));
                    out.push('\n');
                    if hook.expected.is_some() {
                        out.push_str(&muted(&format!("·  {blanks} │  {pad}    └── ")));
                    }
                } else {
                    out.push_str(&muted(&format!("·  {blanks} │  {pad}└── ")));
                }
                if let Some(expected) = &hook.expected {
                    out.push_str(&theme.paint(AuraColor::Pink, "expected:"));
                    out.push(' ');
                    out.push_str(&theme.paint(AuraColor::Text, expected));
                    out.push('\n');
                }
            }
        }
    }
    if hook.footer.is_some() || hook.footer2.is_some() {
        out.push_str(&muted("│"));
        out.push('\n');
    }
    if let Some(footer) = &hook.footer {
        out.push_str(&muted("└─ "));
        out.push_str(footer);
        out.push('\n');
    }
    if let Some(footer2) = &hook.footer2 {
        out.push_str(&muted("└─ "));
        out.push_str(footer2);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::AuraTheme;

    fn sample() -> TreeHook {
        TreeHook {
            title: "error[E042]: type mismatch in variable assignment".to_string(),
            location: Some("src/miner.rnx:18:24".to_string()),
            context: vec![
                (
                    17,
                    "fn calculateRate(base: Float): Float {".to_string(),
                ),
                (18, "    let rate: Float = 42;".to_string()),
                (19, "    return base * rate;".to_string()),
            ],
            pointer_line: 18,
            pointer_col: 22,
            got: Some("Int".to_string()),
            expected: Some("Float".to_string()),
            footer: Some(
                "fix: append '.0' to form a Float literal ('42.0')".to_string(),
            ),
            footer2: None,
        }
    }

    #[test]
    fn plain_box_matches_spec_structure() {
        let out = render(&AuraTheme::plain(), &sample());
        let expected = "┌─ error[E042]: type mismatch in variable assignment\n\
         │  src/miner.rnx:18:24\n\
         │\n\
         │  17 │  fn calculateRate(base: Float): Float {\n\
         │  18 │      let rate: Float = 42;\n\
         ·     │                        ▲\n\
         ·     │                        └──┬── got:      Int\n\
         ·     │                            └── expected: Float\n\
         │  19 │      return base * rate;\n\
         │\n\
         └─ fix: append '.0' to form a Float literal ('42.0')\n";
        assert_eq!(out, expected, "got:\n{out}");
    }

    #[test]
    fn pointer_and_labels_colored() {
        let theme = AuraTheme {
            enabled: true,
            truecolor: true,
        };
        let out = render(&theme, &sample());
        assert!(out.contains("\x1b[38;2;255;103;103m▲\x1b[0m"), "{out}");
        assert!(out.contains("\x1b[38;2;246;148;255mgot:\x1b[0m"), "{out}");
        assert!(out.contains("\x1b[38;2;130;226;255msrc/miner.rnx:18:24\x1b[0m"), "{out}");
    }

    #[test]
    fn minimal_hook_renders() {
        let hook = TreeHook {
            title: "warning[W201]: unrecognized field".to_string(),
            location: None,
            context: vec![],
            pointer_line: 0,
            pointer_col: 0,
            got: None,
            expected: None,
            footer: None,
            footer2: None,
        };
        let out = render(&AuraTheme::plain(), &hook);
        assert_eq!(out, "┌─ warning[W201]: unrecognized field\n");
    }
}
