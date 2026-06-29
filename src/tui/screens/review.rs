use std::sync::mpsc;
use std::time::Instant;
use std::path::PathBuf;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use crossterm::event::{KeyCode, KeyEvent};
use tui_textarea::TextArea;
use crate::models::*;
use crate::render::{cloze::{self, ANSWER_END, ANSWER_START}, html, image as img_render};

pub enum ReviewAction {
    None,
    Rated(Rating, u32),
    Quit,
    Back,
}

enum Side {
    Question,
    Answer(String), // typed answer (may be empty)
}

pub struct ReviewScreen {
    pub card: Card,
    pub note: ResolvedNote,
    side: Side,
    started: Instant,
    active_ord: u32,
    image_cache: img_render::ImageCache,
    /// Pre-rendered halfblock lines cached for current card + area size.
    cached_art: Option<CachedArt>,
    answer_input: TextArea<'static>,
    pending_img: Option<PendingImage>,
}

struct CachedArt {
    src: String,
    cell_w: u16,
    cell_h: u16,
    lines: Vec<Line<'static>>,
}

struct PendingImage {
    src: String,
    cell_w: u16,
    cell_h: u16,
    rx: mpsc::Receiver<(image::DynamicImage, Vec<Line<'static>>)>,
}

impl ReviewScreen {
    pub fn new(card: Card, note: ResolvedNote, media_dir: PathBuf) -> Self {
        let active_ord = (card.ord + 1) as u32;

        let mut answer_input = TextArea::default();
        answer_input.set_block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Your answer — Enter to reveal "),
        );
        answer_input.set_placeholder_text("Type your answer…");

        Self {
            card,
            note,
            side: Side::Question,
            started: Instant::now(),
            active_ord,
            image_cache: img_render::ImageCache::new(media_dir),
            cached_art: None,
            answer_input,
            pending_img: None,
        }
    }

    /// True while the typed-answer TextArea has focus (question side).
    pub fn is_typing(&self) -> bool {
        matches!(&self.side, Side::Question)
    }

    pub fn handle_key(&mut self, key: &KeyEvent) -> ReviewAction {
        match &self.side {
            Side::Question => match key.code {
                // Space skips input only when nothing has been typed yet.
                KeyCode::Char(' ') if self.answer_input.lines().join("").trim().is_empty() => {
                    self.side = Side::Answer(String::new());
                    ReviewAction::None
                }
                // Enter submits the typed answer.
                KeyCode::Enter => {
                    let typed = self.answer_input.lines().join(" ").trim().to_string();
                    self.side = Side::Answer(typed);
                    ReviewAction::None
                }
                KeyCode::Esc => ReviewAction::Back,
                _ => {
                    self.answer_input.input(*key);
                    ReviewAction::None
                }
            },
            Side::Answer(_) => match key.code {
                KeyCode::Char('1') => self.rated(Rating::Again),
                KeyCode::Char('2') => self.rated(Rating::Hard),
                KeyCode::Char('3') => self.rated(Rating::Good),
                KeyCode::Char('4') => self.rated(Rating::Easy),
                KeyCode::Char('q') => ReviewAction::Quit,
                KeyCode::Esc => {
                    self.side = Side::Question;
                    ReviewAction::None
                }
                _ => ReviewAction::None,
            },
        }
    }

    fn rated(&self, rating: Rating) -> ReviewAction {
        let ms = self.started.elapsed().as_millis() as u32;
        ReviewAction::Rated(rating, ms)
    }

    /// Returns the active template for this card (matched by ord).
    fn active_template(&self) -> Option<&crate::models::Template> {
        self.note.notetype.templates
            .iter()
            .find(|t| t.ord == self.card.ord as i32)
    }

    /// Render an Anki template string with field substitution and conditional blocks.
    ///
    /// Handles:
    /// - `{{FieldName}}` → field value
    /// - `{{#FieldName}}...{{/FieldName}}` → show block if field non-empty
    /// - `{{^FieldName}}...{{/FieldName}}` → show block if field empty
    /// - `{{hint:FieldName}}` → field value (hint UI stripped)
    /// - `{{type:FieldName}}` → field value (type-answer stripped)
    fn fill_template(fmt: &str, fields: &[(String, String)]) -> String {
        let map: std::collections::HashMap<&str, &str> =
            fields.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        Self::render_block(fmt, &map)
    }

    fn render_block(s: &str, fields: &std::collections::HashMap<&str, &str>) -> String {
        let mut out = String::new();
        let mut rest = s;
        while let Some(open) = rest.find("{{") {
            out.push_str(&rest[..open]);
            rest = &rest[open..];
            let Some(close) = rest.find("}}") else {
                out.push_str(rest);
                return out;
            };
            let tag = &rest[2..close];
            rest = &rest[close + 2..];

            if let Some(field_name) = tag.strip_prefix('#') {
                // {{#Field}}...{{/Field}} — show if non-empty
                let closing = format!("{{{{/{field_name}}}}}");
                if let Some(end) = rest.find(closing.as_str()) {
                    let inner = &rest[..end];
                    rest = &rest[end + closing.len()..];
                    if !fields.get(field_name).unwrap_or(&"").is_empty() {
                        out.push_str(&Self::render_block(inner, fields));
                    }
                }
            } else if let Some(field_name) = tag.strip_prefix('^') {
                // {{^Field}}...{{/Field}} — show if empty
                let closing = format!("{{{{/{field_name}}}}}");
                if let Some(end) = rest.find(closing.as_str()) {
                    let inner = &rest[..end];
                    rest = &rest[end + closing.len()..];
                    if fields.get(field_name).unwrap_or(&"").is_empty() {
                        out.push_str(&Self::render_block(inner, fields));
                    }
                }
            } else if tag.starts_with('/') {
                // Stray closing tag — ignore
            } else {
                // Simple field: strip hint:/type: prefixes
                let name = tag
                    .strip_prefix("hint:")
                    .or_else(|| tag.strip_prefix("type:"))
                    .unwrap_or(tag)
                    .trim();
                if let Some(val) = fields.get(name) {
                    out.push_str(val);
                }
                // Unknown fields produce nothing (same as Anki behaviour)
            }
        }
        out.push_str(rest);
        out
    }

    /// HTML for the question side: rendered from `qfmt` for standard cards.
    fn question_html(&self) -> String {
        match self.note.notetype.kind {
            NoteKind::Cloze => self.note.first_field().to_string(),
            NoteKind::Standard => match self.active_template() {
                Some(t) => Self::fill_template(&t.qfmt, &self.note.fields),
                None => self.note.first_field().to_string(),
            },
        }
    }

    /// HTML for the answer side: rendered from `afmt` (with FrontSide resolved).
    fn answer_html(&self) -> String {
        match self.note.notetype.kind {
            NoteKind::Cloze => self.note.first_field().to_string(),
            NoteKind::Standard => match self.active_template() {
                Some(t) => {
                    let front = Self::fill_template(&t.qfmt, &self.note.fields);
                    let back = t.afmt.replace("{{FrontSide}}", &front);
                    Self::fill_template(&back, &self.note.fields)
                }
                None => self.note.fields.iter()
                    .map(|(_, v)| v.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
            },
        }
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let side_html = match &self.side {
            Side::Question => self.question_html(),
            Side::Answer(_) => self.answer_html(),
        };
        let (_, srcs) = html::extract(&side_html);
        let has_image = !srcs.is_empty();
        let is_question = matches!(&self.side, Side::Question);
        // Grow height with content: +2 for borders, min 3, max 8.
        let answer_h = if is_question {
            (self.answer_input.lines().len() as u16 + 2).max(3).min(8)
        } else {
            0
        };

        if has_image {
            // Determine image orientation to pick the best layout.
            // Terminal cells are ~2:1 (tall:wide), so account for that: a pixel-square
            // image appears portrait. Portrait → image on right; landscape → image below.
            let portrait = srcs.first()
                .and_then(|s| self.image_cache.dimensions(s))
                .map_or(false, |(w, h)| h > w);

            if portrait {
                // Portrait: full-width header + footer, image left, text right.
                // Footer spans the full width so the image never overlaps the rating keys.
                let outer = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(2),
                        Constraint::Fill(1),
                        Constraint::Length(answer_h),
                        Constraint::Length(2),
                    ])
                    .split(area);
                self.render_header(frame, outer[0]);
                if is_question { frame.render_widget(&self.answer_input, outer[2]); }
                self.render_footer(frame, outer[3]);

                let cols = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
                    .split(outer[1]);

                let content_c = if is_question {
                    Constraint::Fill(1)
                } else {
                    let col_inner_w = ((area.width as u32 * 38 / 100) as u16).saturating_sub(2);
                    let visual_lines: u16 = if col_inner_w == 0 {
                        self.card_text().len() as u16
                    } else {
                        self.card_text().iter().map(|l| {
                            let chars: usize = l.spans.iter().map(|s| s.content.chars().count()).sum();
                            if chars == 0 { 1u16 } else { ((chars as u16 + col_inner_w - 1) / col_inner_w).max(1) }
                        }).sum()
                    };
                    Constraint::Length((visual_lines + 2).min(outer[1].height))
                };
                let text_col = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([content_c])
                    .split(cols[0]);
                self.render_content(frame, text_col[0]);
                self.render_image(frame, cols[1], &srcs);
            } else {
                // Landscape: image at the bottom.
                // Question side: image dominates. Answer side: give content just
                // enough rows for its lines (+2 borders), capped at half the area,
                // so there's no wasted empty block and image fills the rest.
                let (content_c, image_c) = if is_question {
                    (Constraint::Max(6), Constraint::Fill(1))
                } else {
                    let line_count = self.card_text().len() as u16;
                    let h = (line_count + 2).min(area.height / 2);
                    (Constraint::Length(h), Constraint::Fill(1))
                };
                let rows = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(2),
                        content_c,
                        Constraint::Length(answer_h),
                        image_c,
                        Constraint::Length(2),
                    ])
                    .split(area);
                self.render_header(frame, rows[0]);
                self.render_content(frame, rows[1]);
                if is_question { frame.render_widget(&self.answer_input, rows[2]); }
                self.render_image(frame, rows[3], &srcs);
                self.render_footer(frame, rows[4]);
            }
        } else {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(2),
                    Constraint::Fill(1),
                    Constraint::Length(answer_h),
                    Constraint::Length(2),
                ])
                .split(area);
            self.render_header(frame, chunks[0]);
            self.render_content(frame, chunks[1]);
            if is_question { frame.render_widget(&self.answer_input, chunks[2]); }
            self.render_footer(frame, chunks[3]);
        }
    }

    fn poem_title(&self) -> Option<String> {
        self.note.tags.iter()
            .find(|t| t.starts_with("title:"))
            .map(|t| t["title:".len()..].replace('_', " "))
    }

    fn render_header(&self, frame: &mut Frame, area: Rect) {
        let kind = match self.note.notetype.kind {
            NoteKind::Cloze => "cloze",
            NoteKind::Standard => "card",
        };
        let side_label = match &self.side {
            Side::Question => "question",
            Side::Answer(_) => "answer",
        };
        let meta = format!("{} · {} · {}", self.note.notetype.name, kind, side_label);

        if let Some(title) = self.poem_title() {
            // Two-line header: title on top, meta below
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Length(1)])
                .split(area);
            frame.render_widget(
                Paragraph::new(title)
                    .style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                chunks[0],
            );
            frame.render_widget(
                Paragraph::new(meta).style(Style::default().fg(Color::DarkGray)),
                chunks[1],
            );
        } else {
            frame.render_widget(
                Paragraph::new(meta).style(Style::default().fg(Color::DarkGray)),
                area,
            );
        }
    }

    fn render_content(&self, frame: &mut Frame, area: Rect) {
        let text = self.card_text();
        if text.is_empty() || text.iter().all(|l| l.spans.is_empty() || l.spans.iter().all(|s| s.content.trim().is_empty())) {
            return;
        }
        let para = Paragraph::new(text)
            .block(Block::default().borders(Borders::ALL))
            .wrap(Wrap { trim: false });
        frame.render_widget(para, area);
    }

    fn card_text(&self) -> Vec<Line<'static>> {
        match self.note.notetype.kind {
            NoteKind::Cloze => {
                let raw = self.note.first_field();
                let (plain, _) = html::extract(raw);
                let rendered = match &self.side {
                    Side::Question => cloze::render_question(&plain, self.active_ord),
                    Side::Answer(_) => cloze::render_answer(&plain, self.active_ord),
                };
                let mut lines: Vec<Line<'static>> = rendered
                    .lines()
                    .map(|l| styled_cloze_line(l))
                    .collect();
                if let Side::Answer(typed) = &self.side {
                    if !typed.is_empty() {
                        lines.push(Line::from(""));
                        let correct = extract_cloze_answer(&plain, self.active_ord);
                        lines.extend(comparison_lines(typed, &correct));
                    }
                }
                lines
            }
            NoteKind::Standard => match &self.side {
                Side::Question => {
                    let (plain, _) = html::extract(&self.question_html());
                    plain.lines().map(|l| Line::from(l.to_string())).collect()
                }
                Side::Answer(typed) => {
                    let mut lines: Vec<Line<'static>> = Vec::new();
                    // Typed answer comparison against the rendered answer text.
                    if !typed.is_empty() {
                        let answer_html = self.answer_html();
                        let (question_text, _) = html::extract(&self.question_html());
                        let correct = if let Some(t) = self.active_template() {
                            standard_correct(&self.note, &t.qfmt, &answer_html, &question_text)
                        } else {
                            let (answer_text, _) = html::extract(&answer_html);
                            answer_text.lines().next().unwrap_or("").to_string()
                        };
                        lines.extend(comparison_lines(typed, &correct));
                        lines.push(Line::from(Span::styled(
                            "— — —",
                            Style::default().fg(Color::DarkGray),
                        )));
                    }
                    // Use the rendered answer HTML, split at Anki's standard
                    // <hr id=answer> separator so we only show the "back" portion.
                    // If no separator exists, strip {{FrontSide}} from the template.
                    let answer_html = self.answer_html();
                    let back_html: &str = ["<hr id=answer>", "<hr id=\"answer\">", "<hr id='answer'>"]
                        .iter()
                        .find_map(|sep| {
                            answer_html.find(sep).map(|i| &answer_html[i + sep.len()..])
                        })
                        .unwrap_or_else(|| {
                            // No separator — return the slice starting from end so we fall through.
                            &answer_html[answer_html.len()..]
                        });
                    if !back_html.trim().is_empty() {
                        let (plain, _) = html::extract(back_html);
                        for l in plain.lines() {
                            if !l.trim().is_empty() {
                                lines.push(Line::from(l.to_string()));
                            }
                        }
                    } else {
                        // No <hr id=answer> — fall back to all non-empty raw fields.
                        for (name, val) in &self.note.fields {
                            let (plain, _) = html::extract(val);
                            if !plain.is_empty() {
                                lines.push(Line::from(Span::styled(
                                    format!("{name}:"),
                                    Style::default()
                                        .fg(Color::Cyan)
                                        .add_modifier(Modifier::BOLD),
                                )));
                                for l in plain.lines() {
                                    lines.push(Line::from(l.to_string()));
                                }
                            }
                        }
                    }
                    lines
                }
            },
        }
    }

    fn render_image(&mut self, frame: &mut Frame, area: Rect, srcs: &[String]) {
        let Some(src) = srcs.first() else { return };
        let (cell_w, cell_h) = (area.width, area.height);

        // Fast path: cached art is current.
        // Height tolerance of 10 rows prevents forced re-renders caused by the answer
        // input box appearing/disappearing between question and answer sides.
        let cached_ok = self.cached_art.as_ref().map_or(false, |c| {
            c.src == *src && c.cell_w == cell_w && c.cell_h.abs_diff(cell_h) <= 10
        });
        if cached_ok {
            let lines = self.cached_art.as_ref().unwrap().lines.clone();
            frame.render_widget(Paragraph::new(lines), area);
            return;
        }

        // Check whether the background thread has finished.
        let ready = if let Some(p) = &self.pending_img {
            p.src == *src && p.cell_w == cell_w && p.cell_h == cell_h
        } else {
            false
        };

        if ready {
            match self.pending_img.as_ref().unwrap().rx.try_recv() {
                Ok((raw_img, lines)) => {
                    self.image_cache.store(src, raw_img);
                    self.cached_art = Some(CachedArt { src: src.clone(), cell_w, cell_h, lines });
                    self.pending_img = None;
                    let lines = self.cached_art.as_ref().unwrap().lines.clone();
                    frame.render_widget(Paragraph::new(lines), area);
                    return;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    // Still loading — fall through to placeholder.
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    // Thread failed (file not found, decode error).
                    self.pending_img = None;
                    frame.render_widget(
                        Paragraph::new(format!("[image: {src}]"))
                            .style(Style::default().fg(Color::DarkGray)),
                        area,
                    );
                    return;
                }
            }
        }

        // Nothing ready yet — if no thread is running, start one.
        let needs_spawn = self.pending_img.as_ref()
            .map_or(true, |p| p.src != *src || p.cell_w != cell_w || p.cell_h != cell_h);

        if needs_spawn {
            // Check LRU cache first; if already decoded, only quadrant conversion needed.
            if let Some(img) = self.image_cache.get(src) {
                // Raw image is cached: just re-convert for new dimensions.
                let (w, h) = img_render::fit_dimensions(&img, cell_w, cell_h);
                let lines = img_render::to_quadrant_blocks(&img, w, h);
                self.cached_art = Some(CachedArt { src: src.clone(), cell_w, cell_h, lines });
                let lines = self.cached_art.as_ref().unwrap().lines.clone();
                frame.render_widget(Paragraph::new(lines), area);
                return;
            }

            // Not cached — load from disk in a background thread.
            let (tx, rx) = mpsc::channel();
            let src_clone = src.clone();
            let media_dir = self.image_cache.media_dir.clone();
            std::thread::spawn(move || {
                let path = media_dir.join(&src_clone);
                if let Some(img) = img_render::load_from_disk(&path) {
                    let (w, h) = img_render::fit_dimensions(&img, cell_w, cell_h);
                    let lines = img_render::to_quadrant_blocks(&img, w, h);
                    let _ = tx.send((img, lines));
                }
            });
            self.pending_img = Some(PendingImage { src: src.clone(), cell_w, cell_h, rx });
        }

        // Show placeholder while loading.
        frame.render_widget(
            Paragraph::new("  loading image…")
                .style(Style::default().fg(Color::DarkGray)),
            area,
        );
    }

    fn render_footer(&self, frame: &mut Frame, area: Rect) {
        let text = match &self.side {
            Side::Question => Line::from(vec![
                Span::styled("[Enter]", Style::default().fg(Color::Yellow)),
                Span::raw(" Reveal  "),
                Span::styled("[Space]", Style::default().fg(Color::DarkGray)),
                Span::raw(" Skip input  "),
                Span::styled("[Esc]", Style::default().fg(Color::DarkGray)),
                Span::raw(" Back  "),
                Span::styled("[c]", Style::default().fg(Color::Cyan)),
                Span::raw(" Create  "),
                Span::styled("[a]", Style::default().fg(Color::Cyan)),
                Span::raw(" AI  "),
                Span::styled("[p]", Style::default().fg(Color::Cyan)),
                Span::raw(" Poem"),
            ]),
            Side::Answer(_) => Line::from(vec![
                Span::styled("[1]", Style::default().fg(Color::Red)),
                Span::raw(" Again  "),
                Span::styled("[2]", Style::default().fg(Color::Yellow)),
                Span::raw(" Hard  "),
                Span::styled("[3]", Style::default().fg(Color::Green)),
                Span::raw(" Good  "),
                Span::styled("[4]", Style::default().fg(Color::Cyan)),
                Span::raw(" Easy  "),
                Span::styled("[Esc]", Style::default().fg(Color::DarkGray)),
                Span::raw(" Re-read"),
            ]),
        };
        let para = Paragraph::new(text)
            .block(Block::default().borders(Borders::TOP));
        frame.render_widget(para, area);
    }
}

/// Determine the correct answer for a standard card's typed-answer comparison.
///
/// Priority:
///   1. `{{type:FieldName}}` in qfmt → that field's value (Anki native type-answer mode)
///   2. First field whose name does NOT appear as `{{FieldName}}` in qfmt
///      (the question shows certain fields; the answer is a field not shown)
///   3. Heuristic: first line of rendered answer that differs from the question
fn standard_correct(note: &ResolvedNote, qfmt: &str, answer_html: &str, question_text: &str) -> String {
    use once_cell::sync::Lazy;
    use regex::Regex;
    use std::collections::HashSet;

    // Priority 1: explicit {{type:FieldName}}
    static TYPE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\{\{type:(.+?)\}\}").unwrap());
    if let Some(cap) = TYPE_RE.captures(qfmt) {
        let fname = cap[1].trim();
        if let Some((_, val)) = note.fields.iter().find(|(n, _)| n == fname) {
            let (plain, _) = html::extract(val);
            if !plain.trim().is_empty() {
                return plain;
            }
        }
    }

    // Priority 2: first field not referenced (by plain {{Name}}) in qfmt.
    // We look for {{Name}} patterns that don't have a prefix like "type:" or "#".
    static REF_RE: Lazy<Regex> = Lazy::new(|| {
        // Matches {{Name}} where Name contains no colon (excludes type:, hint:, etc.)
        Regex::new(r"\{\{([^}:]+)\}\}").unwrap()
    });
    let shown: HashSet<String> = REF_RE.captures_iter(qfmt)
        .map(|c| c[1].trim().to_string())
        .filter(|s| !s.starts_with('#') && !s.starts_with('^') && !s.starts_with('/'))
        .collect();
    if !shown.is_empty() {
        for (name, val) in &note.fields {
            if !shown.contains(name.as_str()) {
                let (plain, _) = html::extract(val);
                if !plain.trim().is_empty() {
                    return plain;
                }
            }
        }
    }

    // Priority 3: heuristic — first non-empty answer line not in question
    let (answer_text, _) = html::extract(answer_html);
    answer_text.lines()
        .find(|l| {
            let l = l.trim();
            !l.is_empty() && !question_text.contains(l)
        })
        .unwrap_or(answer_text.lines().next().unwrap_or(""))
        .to_string()
}

fn extract_cloze_answer(text: &str, active_ord: u32) -> String {
    use once_cell::sync::Lazy;
    use regex::Regex;
    static RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"\{\{c(\d+)::(.+?)(?:::(.+?))?\}\}").unwrap()
    });
    RE.captures_iter(text)
        .find(|c| c[1].parse::<u32>().ok() == Some(active_ord))
        .map(|c| c[2].trim().to_string())
        .unwrap_or_default()
}

/// Convert a raw rendered cloze line (which may contain sentinel markers) into a styled Line.
fn styled_cloze_line(raw: &str) -> Line<'static> {
    if let (Some(s), Some(e)) = (raw.find(ANSWER_START), raw.find(ANSWER_END)) {
        let before = raw[..s].to_string();
        let answer = raw[s + ANSWER_START.len()..e].to_string();
        let after = raw[e + ANSWER_END.len()..].to_string();
        Line::from(vec![
            Span::raw(before),
            Span::styled(answer, Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::raw(after),
        ])
    } else {
        Line::from(raw.to_string())
    }
}

fn comparison_lines(typed: &str, correct: &str) -> Vec<Line<'static>> {
    let matched = typed.trim().to_lowercase() == correct.trim().to_lowercase();
    if matched {
        return vec![Line::from(vec![
            Span::styled("✓ ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::raw(typed.to_string()),
        ])];
    }

    // Word-level diff using dissimilar
    let chunks = dissimilar::diff(correct.trim(), typed.trim());
    let mut typed_spans: Vec<Span<'static>> = vec![
        Span::styled("✗ typed:   ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
    ];
    let mut correct_spans: Vec<Span<'static>> = vec![
        Span::styled("  correct: ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
    ];
    for chunk in &chunks {
        match chunk {
            dissimilar::Chunk::Equal(s) => {
                typed_spans.push(Span::raw(s.to_string()));
                correct_spans.push(Span::raw(s.to_string()));
            }
            dissimilar::Chunk::Delete(s) => {
                // In correct but not in typed — highlight missing in correct line
                correct_spans.push(Span::styled(
                    s.to_string(),
                    Style::default().fg(Color::Green).add_modifier(Modifier::UNDERLINED),
                ));
            }
            dissimilar::Chunk::Insert(s) => {
                // In typed but not in correct — highlight extra in typed line
                typed_spans.push(Span::styled(
                    s.to_string(),
                    Style::default().fg(Color::Red).add_modifier(Modifier::UNDERLINED),
                ));
            }
        }
    }
    vec![Line::from(typed_spans), Line::from(correct_spans)]
}
