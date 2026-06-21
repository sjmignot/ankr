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
use ratatui_image::{picker::Picker, StatefulImage, protocol::StatefulProtocol};
use crate::models::*;
use crate::render::{cloze, html};

pub enum ReviewAction {
    None,
    Rated(Rating, u32),       // (rating, ms elapsed)
    Quit,
    Back,
}

enum Side { Question, Answer }

pub struct ReviewScreen {
    pub card: Card,
    pub note: ResolvedNote,
    side: Side,
    started: Instant,
    active_ord: u32,         // 1-indexed cloze ordinal
    image_proto: Option<StatefulProtocol>,
    picker: Option<Picker>,
    media_dir: PathBuf,
    image_cache: crate::render::image::ImageCache,
}

impl ReviewScreen {
    pub fn new(
        card: Card,
        note: ResolvedNote,
        picker: Option<Picker>,
        media_dir: PathBuf,
    ) -> Self {
        let active_ord = (card.ord + 1) as u32;
        let image_cache = crate::render::image::ImageCache::new(media_dir.clone());
        Self {
            card,
            note,
            side: Side::Question,
            started: Instant::now(),
            active_ord,
            image_proto: None,
            picker,
            media_dir,
            image_cache,
        }
    }

    pub fn handle_key(&mut self, key: &KeyEvent) -> ReviewAction {
        match &self.side {
            Side::Question => match key.code {
                KeyCode::Char(' ') | KeyCode::Enter => {
                    self.side = Side::Answer;
                    self.image_proto = None; // reset so answer image loads fresh
                    ReviewAction::None
                }
                KeyCode::Char('q') => ReviewAction::Quit,
                KeyCode::Esc | KeyCode::Char('b') => ReviewAction::Back,
                _ => ReviewAction::None,
            },
            Side::Answer => match key.code {
                KeyCode::Char('1') => self.rated(Rating::Again),
                KeyCode::Char('2') => self.rated(Rating::Hard),
                KeyCode::Char('3') => self.rated(Rating::Good),
                KeyCode::Char('4') => self.rated(Rating::Easy),
                KeyCode::Char('q') => ReviewAction::Quit,
                KeyCode::Esc => { self.side = Side::Question; ReviewAction::None }
                _ => ReviewAction::None,
            },
        }
    }

    fn rated(&self, rating: Rating) -> ReviewAction {
        let ms = self.started.elapsed().as_millis() as u32;
        ReviewAction::Rated(rating, ms)
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),  // header
                Constraint::Min(4),     // card content
                Constraint::Length(4),  // image area or padding
                Constraint::Length(3),  // footer / ratings
            ])
            .split(area);

        self.render_header(frame, chunks[0]);
        self.render_content(frame, chunks[1]);
        self.render_image(frame, chunks[2]);
        self.render_footer(frame, chunks[3]);
    }

    fn render_header(&self, frame: &mut Frame, area: Rect) {
        let kind = match self.note.notetype.kind {
            NoteKind::Cloze => "cloze",
            NoteKind::Standard => "card",
        };
        let side_label = match self.side {
            Side::Question => "question",
            Side::Answer   => "answer",
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
                let rendered = match self.side {
                    Side::Question => cloze::render_question(&plain, self.active_ord),
                    Side::Answer   => cloze::render_answer(&plain, self.active_ord),
                };
                rendered.lines().map(|l| Line::from(l.to_string())).collect()
            }
            NoteKind::Standard => {
                match self.side {
                    Side::Question => {
                        let (plain, _) = html::extract(self.note.first_field());
                        plain.lines().map(|l| Line::from(l.to_string())).collect()
                    }
                    Side::Answer => {
                        let mut lines: Vec<Line> = Vec::new();
                        lines.push(Line::from(Span::styled(
                            "— — —",
                            Style::default().fg(Color::DarkGray),
                        )));
                        for (name, val) in &self.note.fields {
                            let (plain, _) = html::extract(val);
                            if !plain.is_empty() {
                                lines.push(Line::from(Span::styled(
                                    format!("{name}:"),
                                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                                )));
                                for l in plain.lines() {
                                    lines.push(Line::from(l.to_string()));
                                }
                            }
                        }
                        lines
                    }
                }
            }
        }
    }

    fn render_image(&mut self, frame: &mut Frame, area: Rect) {
        // Collect image srcs from all fields.
        let srcs: Vec<String> = self.note.fields.iter()
            .flat_map(|(_, v)| html::extract(v).1)
            .collect();

        if let (Some(src), Some(picker)) = (srcs.first(), &self.picker) {
            if self.image_proto.is_none() {
                if let Some(img) = self.image_cache.get(src) {
                    self.image_proto = Some(picker.new_resize_protocol(img));
                }
            }
            if let Some(proto) = &mut self.image_proto {
                frame.render_stateful_widget(StatefulImage::default(), area, proto);
                return;
            }
        }
        // No image — render blank
        frame.render_widget(Paragraph::new(""), area);
    }

    fn render_footer(&self, frame: &mut Frame, area: Rect) {
        let text = match self.side {
            Side::Question => Line::from(vec![
                Span::styled("[Space]", Style::default().fg(Color::Yellow)),
                Span::raw(" Reveal  "),
                Span::styled("[q]", Style::default().fg(Color::DarkGray)),
                Span::raw(" Quit"),
            ]),
            Side::Answer => Line::from(vec![
                Span::styled("[1]", Style::default().fg(Color::Red)),
                Span::raw(" Again  "),
                Span::styled("[2]", Style::default().fg(Color::Yellow)),
                Span::raw(" Hard  "),
                Span::styled("[3]", Style::default().fg(Color::Green)),
                Span::raw(" Good  "),
                Span::styled("[4]", Style::default().fg(Color::Cyan)),
                Span::raw(" Easy"),
            ]),
        };
        let para = Paragraph::new(text)
            .block(Block::default().borders(Borders::TOP));
        frame.render_widget(para, area);
    }
}
