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

/// Internal sentinel wrapping the revealed cloze answer.
/// review.rs detects this and applies styling; it is never shown raw.
pub const ANSWER_START: &str = "\x02";
pub const ANSWER_END: &str = "\x03";

/// Render the answer side: active ordinal is shown with surrounding markers.
pub fn render_answer(text: &str, active_ord: u32) -> String {
    CLOZE_RE.replace_all(text, |caps: &regex::Captures| {
        let n: u32 = caps[1].parse().unwrap_or(0);
        if n == active_ord {
            format!("{}{}{}", ANSWER_START, &caps[2], ANSWER_END)
        } else {
            caps[2].to_string()
        }
    }).to_string()
}

