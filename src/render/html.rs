/// Strip HTML tags and decode entities from Anki field content.
pub fn strip(html: &str) -> String {
    if !html.contains('<') {
        // Plain text — preserve newlines as-is.
        return html.trim().to_string();
    }
    // Replace <a href="url">text</a> → "text (domain)" — shows where the
    // link goes without a long URL that wraps badly in narrow columns.
    let with_domains = {
        let re = regex::Regex::new(r#"(?si)<a\b[^>]*\bhref\s*=\s*"([^"]*)"[^>]*>(.*?)</a>"#).unwrap();
        re.replace_all(html, |caps: &regex::Captures| {
            let url = caps[1].trim();
            let text = caps[2].trim();
            let url_display = url
                .trim_start_matches("https://")
                .trim_start_matches("http://");
            if url_display.is_empty() { text.to_string() } else { format!("{text} ({url_display})") }
        }).to_string()
    };
    // Strip any remaining <a> tags (named anchors without href).
    let no_anchors = regex::Regex::new(r#"(?si)<a\b[^>]*>(.*?)</a>"#)
        .unwrap()
        .replace_all(&with_domains, "$1")
        .to_string();
    html2text::from_read(no_anchors.as_bytes(), 10000)
        .unwrap_or_else(|_| html.to_string())
        .lines()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// Returns (plain text, list of <img src="..."> paths) from an HTML field.
pub fn extract(html: &str) -> (String, Vec<String>) {
    let srcs = crate::render::image::extract_srcs(html);
    let without_imgs = regex::Regex::new(r#"<img[^>]*>"#)
        .unwrap()
        .replace_all(html, "")
        .to_string();
    (strip(&without_imgs), srcs)
}
