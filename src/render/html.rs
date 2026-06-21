/// Strip HTML tags and decode entities from Anki field content.
pub fn strip(html: &str) -> String {
    html2text::from_read(html.as_bytes(), 10000)
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
