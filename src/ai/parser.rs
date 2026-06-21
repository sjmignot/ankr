use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;
use crate::error::{AnkrError, Result};
use crate::models::NewCard;

static CODE_BLOCK_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"```(?:json)?\s*([\s\S]*?)\s*```").unwrap()
});

#[derive(Deserialize)]
struct RawCard {
    text: String,
    #[serde(default)]
    tags: Vec<String>,
}

pub fn parse_cards(content: &str, deck_id: i64, notetype_id: i64) -> Result<Vec<NewCard>> {
    // Strip markdown code block if present
    let json_str = if let Some(cap) = CODE_BLOCK_RE.captures(content) {
        cap[1].trim().to_string()
    } else {
        // Try to find a JSON array in the raw text
        let start = content.find('[').unwrap_or(0);
        let end = content.rfind(']').map(|i| i + 1).unwrap_or(content.len());
        content[start..end].trim().to_string()
    };

    let raw: Vec<RawCard> = serde_json::from_str(&json_str)
        .map_err(|e| AnkrError::Ai(format!("parsing card JSON: {e}\nContent: {json_str}")))?;

    Ok(raw.into_iter().map(|r| NewCard {
        text: r.text,
        back: String::new(),
        tags: r.tags,
        deck_id,
        notetype_id,
    }).collect())
}
