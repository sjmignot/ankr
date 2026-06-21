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
}

struct CachedArt {
    src: String,
    cell_w: u16,
    cell_h: u16,
    lines: Vec<Line<'static>>,
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

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let srcs: Vec<String> = self.note.fields.iter()
            .flat_map(|(_, v)| img_render::extract_srcs(v))
            .collect();
        let has_image = !srcs.is_empty();
        let is_question = matches!(&self.side, Side::Question);
        let answer_h = if is_question { 3u16 } else { 0 };

        if has_image {
            // Determine image orientation to pick the best layout.
            // Terminal cells are ~2:1 (tall:wide), so account for that: a pixel-square
            // image appears portrait. Portrait → image on right; landscape → image below.
            let portrait = srcs.first()
                .and_then(|s| self.image_cache.dimensions(s))
                .map_or(false, |(w, h)| h > w);

            if portrait {
                // Portrait: image on the right, controls on the left.
                let cols = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
                    .split(area);
                let left = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(2),
                        Constraint::Fill(1),
                        Constraint::Length(answer_h),
                        Constraint::Length(2),
                    ])
                    .split(cols[0]);
                self.render_header(frame, left[0]);
                self.render_content(frame, left[1]);
                if is_question { frame.render_widget(&self.answer_input, left[2]); }
                self.render_footer(frame, left[3]);
                self.render_image(frame, cols[1], &srcs);
            } else {
                // Landscape: image at the bottom, fills remaining height.
                let rows = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(2),
                        Constraint::Max(6),
                        Constraint::Length(answer_h),
                        Constraint::Fill(1),
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
                    let (plain, _) = html::extract(self.note.first_field());
                    plain.lines().map(|l| Line::from(l.to_string())).collect()
                }
                Side::Answer(typed) => {
                    let mut lines: Vec<Line<'static>> = Vec::new();
                    if !typed.is_empty() {
                        let correct_html = self.note.fields.get(1)
                            .map(|(_, v)| v.as_str())
                            .unwrap_or("");
                        let (correct, _) = html::extract(correct_html);
                        lines.extend(comparison_lines(typed, &correct));
                        lines.push(Line::from(Span::styled(
                            "— — —",
                            Style::default().fg(Color::DarkGray),
                        )));
                    }
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
                    lines
                }
            },
        }
    }

    fn render_image(&mut self, frame: &mut Frame, area: Rect, srcs: &[String]) {
        let Some(src) = srcs.first() else { return };
        let (cell_w, cell_h) = (area.width, area.height);

        // Regenerate if src changed or area resized.
        let stale = self.cached_art.as_ref().map_or(true, |c| {
            c.src != *src || c.cell_w != cell_w || c.cell_h != cell_h
        });

        if stale {
            if let Some(img) = self.image_cache.get(src) {
                let (w, h) = img_render::fit_dimensions(&img, cell_w, cell_h);
                let lines = img_render::to_quadrant_blocks(&img, w, h);
                self.cached_art = Some(CachedArt {
                    src: src.clone(),
                    cell_w,
                    cell_h,
                    lines,
                });
            }
        }

        if let Some(art) = &self.cached_art {
            let para = Paragraph::new(art.lines.clone());
            frame.render_widget(para, area);
        } else {
            // Image not found — show the filename so user knows what's missing.
            let para = Paragraph::new(format!("[image: {src}]"))
                .style(Style::default().fg(Color::DarkGray));
            frame.render_widget(para, area);
        }
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

fn extract_cloze_answer(text: &str, active_ord: u32) -> String {
    use once_cell::sync::Lazy;
    use regex::Regex;
    static RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"\{\{c(\d+)::([^:}]+)(?:::([^}]+))?\}\}").unwrap()
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
