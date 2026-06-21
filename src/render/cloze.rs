use once_cell::sync::Lazy;
use regex::Regex;

static CLOZE_RE: Lazy<Regex> = Lazy::new(|| {
    // Matches {{cN::answer}} or {{cN::answer::hint}}
    Regex::new(r"\{\{c(\d+)::([^:}]+)(?:::([^}]+))?\}\}").unwrap()
});

/// Render the question side: the active ordinal is masked; others are shown plainly.
/// `active_ord` is 1-indexed (card.ord + 1).
pub fn render_question(text: &str, active_ord: u32) -> String {
    CLOZE_RE.replace_all(text, |caps: &regex::Captures| {
        let n: u32 = caps[1].parse().unwrap_or(0);
        if n == active_ord {
            let hint = caps.get(3).map(|m| m.as_str());
            match hint {
                Some(h) => format!("[{h}]"),
                None => "[...]".to_string(),
            }
        } else {
            caps[2].to_string()
        }
    }).to_string()
}

/// Render the answer side: active ordinal is shown with surrounding markers.
pub fn render_answer(text: &str, active_ord: u32) -> String {
    CLOZE_RE.replace_all(text, |caps: &regex::Captures| {
        let n: u32 = caps[1].parse().unwrap_or(0);
        if n == active_ord {
            format!(">>{}<<", &caps[2])
        } else {
            caps[2].to_string()
        }
    }).to_string()
}

/// Returns true if the field contains any cloze deletions.
pub fn has_cloze(text: &str) -> bool {
    CLOZE_RE.is_match(text)
}

/// Collects all unique cloze ordinals (1-indexed) in the text.
pub fn cloze_ords(text: &str) -> Vec<u32> {
    let mut ords: Vec<u32> = CLOZE_RE.captures_iter(text)
        .filter_map(|c| c[1].parse().ok())
        .collect();
    ords.sort_unstable();
    ords.dedup();
    ords
}
