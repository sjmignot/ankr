use std::time::Instant;
use std::path::PathBuf;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui_image::{picker::Picker, StatefulImage, protocol::StatefulProtocol};
use tui_textarea::TextArea;
use crate::models::*;
use crate::render::{cloze, html, image as img_render};

pub enum ReviewAction {
    None,
    Rated(Rating, u32),
    Quit,
    Back,
}

enum Side {
    Question,
    /// Holds user's typed answer (empty string = no input field was shown)
    Answer(String),
}

pub struct ReviewScreen {
    pub card: Card,
    pub note: ResolvedNote,
    side: Side,
    started: Instant,
    active_ord: u32,
    image_cache: img_render::ImageCache,
    image_proto: Option<StatefulProtocol>,
    picker: Picker,
    /// Text input shown on the question side for the user to type an answer.
    answer_input: TextArea<'static>,
}

impl ReviewScreen {
    pub fn new(card: Card, note: ResolvedNote, media_dir: PathBuf) -> Self {
        let active_ord = (card.ord + 1) as u32;
        let picker = Picker::from_query_stdio()
            .unwrap_or_else(|_| Picker::halfblocks());

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
            image_proto: None,
            picker,
            answer_input,
        }
    }

    pub fn handle_key(&mut self, key: &KeyEvent) -> ReviewAction {
        match &self.side {
            Side::Question => match key.code {
                // Enter/Space when answer box is empty reveals directly.
                // Enter when answer box has text reveals with comparison.
                KeyCode::Enter | KeyCode::Char(' ')
                    if key.code == KeyCode::Char(' ')
                        || self.answer_input.lines().iter().all(|l| l.is_empty()) =>
                {
                    let typed = self.answer_input.lines().join(" ").trim().to_string();
                    self.side = Side::Answer(typed);
                    self.image_proto = None;
                    ReviewAction::None
                }
                KeyCode::Enter => {
                    let typed = self.answer_input.lines().join(" ").trim().to_string();
                    self.side = Side::Answer(typed);
                    self.image_proto = None;
                    ReviewAction::None
                }
                KeyCode::Char('q') if key.modifiers.is_empty() => ReviewAction::Quit,
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
            .flat_map(|(_, v)| html::extract(v).1)
            .collect();
        let has_image = !srcs.is_empty();

        let image_height = if has_image { 20u16 } else { 0 };
        let answer_height = match &self.side {
            Side::Question => 3u16,
            Side::Answer(_) => 0,
        };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),              // header
                Constraint::Min(3),                 // card text
                Constraint::Length(answer_height),  // answer input (question side only)
                Constraint::Length(image_height),   // image
                Constraint::Length(3),              // footer / ratings
            ])
            .split(area);

        self.render_header(frame, chunks[0]);
        self.render_content(frame, chunks[1]);

        if let Side::Question = &self.side {
            frame.render_widget(&self.answer_input, chunks[2]);
        }

        if has_image {
            self.render_image(frame, chunks[3], &srcs);
        }
        self.render_footer(frame, chunks[4]);
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
        let header = Paragraph::new(format!(
            "{} · {} · {}",
            self.note.notetype.name, kind, side_label
        ))
        .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(header, area);
    }

    fn render_content(&self, frame: &mut Frame, area: Rect) {
        let text = self.card_text();
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
                    .map(|l| Line::from(l.to_string()))
                    .collect();

                // On the answer side, show typed answer vs correct answer.
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

                    // Typed answer comparison (if user typed something).
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

                    // All fields.
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

        if self.image_proto.is_none() {
            if let Some(img) = self.image_cache.get(src) {
                self.image_proto = Some(self.picker.new_resize_protocol(img));
            }
        }

        if let Some(proto) = &mut self.image_proto {
            frame.render_stateful_widget(StatefulImage::default(), area, proto);
        }
    }

    fn render_footer(&self, frame: &mut Frame, area: Rect) {
        let text = match &self.side {
            Side::Question => Line::from(vec![
                Span::styled("[Enter]", Style::default().fg(Color::Yellow)),
                Span::raw(" Reveal  "),
                Span::styled("[Space]", Style::default().fg(Color::Yellow)),
                Span::raw(" Reveal (skip input)  "),
                Span::styled("[q]", Style::default().fg(Color::DarkGray)),
                Span::raw(" Quit  "),
                Span::styled("[c]", Style::default().fg(Color::Cyan)),
                Span::raw(" Create  "),
                Span::styled("[a]", Style::default().fg(Color::Cyan)),
                Span::raw(" AI"),
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

/// Extract the answer text for a specific cloze ordinal.
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

/// Build lines comparing what the user typed vs the correct answer.
fn comparison_lines(typed: &str, correct: &str) -> Vec<Line<'static>> {
    let typed_lc = typed.trim().to_lowercase();
    let correct_lc = correct.trim().to_lowercase();
    let correct_match = typed_lc == correct_lc;

    let (icon, color) = if correct_match {
        ("✓", Color::Green)
    } else {
        ("✗", Color::Red)
    };

    let mut lines = vec![
        Line::from(vec![
            Span::styled(format!("{icon} You: "), Style::default().fg(color).add_modifier(Modifier::BOLD)),
            Span::raw(typed.to_string()),
        ]),
    ];
    if !correct_match {
        lines.push(Line::from(vec![
            Span::styled("  Answer: ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::raw(correct.to_string()),
        ]));
    }
    lines
}
