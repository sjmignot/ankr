#[derive(Clone, Copy, PartialEq)]
pub enum GranularityMode {
    /// LPCG line mode: 2 preceding lines as context, blank target line.
    Line,
    /// LPCG stanza mode: 1 preceding stanza as context, blank target stanza.
    Stanza,
}

impl GranularityMode {
    pub fn toggle(self) -> Self {
        match self {
            Self::Line => Self::Stanza,
            Self::Stanza => Self::Line,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Line => "Lines (2 ctx)",
            Self::Stanza => "Stanzas",
        }
    }
}

/// Generate LPCG-style cloze cards from a poem.
///
/// Each returned string is a self-contained cloze note:
/// the preceding context lines/stanza (plain text) followed by the target
/// unit wrapped in `{{c1::...}}`. One note → one card.
///
/// Line mode example (context=2):
///   "Shall I compare thee to a summer's day?\nThou art more lovely...\n{{c1::Rough winds...}}"
///
/// Stanza mode example:
///   "First stanza text\n\n{{c1::Second stanza text}}"
pub fn poem_to_lpcg(text: &str, mode: GranularityMode) -> Vec<String> {
    let text = text.trim_end_matches('\n');
    match mode {
        GranularityMode::Line => {
            let lines: Vec<&str> = text
                .split('\n')
                .filter(|l| !l.trim().is_empty())
                .collect();
            lines
                .iter()
                .enumerate()
                .map(|(i, line)| {
                    let ctx_start = i.saturating_sub(2);
                    let mut parts: Vec<String> =
                        lines[ctx_start..i].iter().map(|l| l.to_string()).collect();
                    parts.push(format!("{{{{c1::{line}}}}}"));
                    parts.join("\n")
                })
                .collect()
        }
        GranularityMode::Stanza => {
            let stanzas: Vec<&str> = text
                .split("\n\n")
                .filter(|s| !s.trim().is_empty())
                .collect();
            stanzas
                .iter()
                .enumerate()
                .map(|(i, stanza)| {
                    if i == 0 {
                        format!("{{{{c1::{stanza}}}}}")
                    } else {
                        format!("{}\n\n{{{{c1::{stanza}}}}}", stanzas[i - 1])
                    }
                })
                .collect()
        }
    }
}

/// Number of cards that would be generated (= non-empty lines or stanzas).
pub fn count_cards(text: &str, mode: GranularityMode) -> usize {
    let text = text.trim_end_matches('\n');
    match mode {
        GranularityMode::Line => text.split('\n').filter(|l| !l.trim().is_empty()).count(),
        GranularityMode::Stanza => text.split("\n\n").filter(|s| !s.trim().is_empty()).count(),
    }
}
