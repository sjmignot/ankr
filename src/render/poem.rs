#[derive(Clone, Copy, PartialEq)]
pub enum GranularityMode {
    Line,
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
            Self::Line => "Line",
            Self::Stanza => "Stanza",
        }
    }
}

/// Wrap each non-empty line (Line mode) or stanza block (Stanza mode) in
/// `{{cN::...}}` cloze syntax. Blank lines are preserved as separators.
pub fn poem_to_cloze(text: &str, mode: GranularityMode) -> String {
    let text = text.trim_end_matches('\n');
    match mode {
        GranularityMode::Line => {
            let mut n = 1u32;
            text.split('\n')
                .map(|line| {
                    if line.trim().is_empty() {
                        line.to_string()
                    } else {
                        let out = format!("{{{{c{n}::{line}}}}}");
                        n += 1;
                        out
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        GranularityMode::Stanza => {
            let mut n = 1u32;
            text.split("\n\n")
                .map(|stanza| {
                    if stanza.trim().is_empty() {
                        String::new()
                    } else {
                        let out = format!("{{{{c{n}::{stanza}}}}}");
                        n += 1;
                        out
                    }
                })
                .collect::<Vec<_>>()
                .join("\n\n")
        }
    }
}

pub fn count_cloze_units(text: &str, mode: GranularityMode) -> usize {
    let text = text.trim_end_matches('\n');
    match mode {
        GranularityMode::Line => text.split('\n').filter(|l| !l.trim().is_empty()).count(),
        GranularityMode::Stanza => text.split("\n\n").filter(|s| !s.trim().is_empty()).count(),
    }
}
