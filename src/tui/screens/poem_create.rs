use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tui_textarea::TextArea;
use crate::models::NewCard;
use crate::render::poem::{GranularityMode, count_cards, poem_to_lpcg};

pub enum PoemCreateAction {
    None,
    Save(Vec<NewCard>),
    Cancel,
}

#[derive(Clone, Copy, PartialEq)]
enum PoemFocus { Poem, Author, Tags }

pub struct PoemCreateScreen {
    poem_area: TextArea<'static>,
    author_area: TextArea<'static>,
    tags_area: TextArea<'static>,
    focus: PoemFocus,
    mode: GranularityMode,
    deck_id: i64,
    notetype_id: i64,
}

impl PoemCreateScreen {
    pub fn new(deck_id: i64, notetype_id: i64) -> Self {
        let mut poem_area = TextArea::default();
        poem_area.set_block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Poem text "),
        );
        poem_area.set_placeholder_text("Shall I compare thee to a summer's day?");

        let mut author_area = TextArea::default();
        author_area.set_block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Author "),
        );
        author_area.set_placeholder_text("Shakespeare");

        let mut tags_area = TextArea::default();
        tags_area.set_block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Tags (space-separated) "),
        );
        tags_area.set_placeholder_text("sonnets poem");

        Self {
            poem_area,
            author_area,
            tags_area,
            focus: PoemFocus::Poem,
            mode: GranularityMode::Line,
            deck_id,
            notetype_id,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> PoemCreateAction {
        match key.code {
            KeyCode::Esc => return PoemCreateAction::Cancel,
            KeyCode::Tab => {
                self.focus = match self.focus {
                    PoemFocus::Poem => PoemFocus::Author,
                    PoemFocus::Author => PoemFocus::Tags,
                    PoemFocus::Tags => PoemFocus::Poem,
                };
                return PoemCreateAction::None;
            }
            KeyCode::BackTab => {
                self.focus = match self.focus {
                    PoemFocus::Poem => PoemFocus::Tags,
                    PoemFocus::Tags => PoemFocus::Author,
                    PoemFocus::Author => PoemFocus::Poem,
                };
                return PoemCreateAction::None;
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return self.build_cards();
            }
            KeyCode::Enter if self.focus == PoemFocus::Tags || self.focus == PoemFocus::Author => {
                return self.build_cards();
            }
            KeyCode::Char('g') if self.focus == PoemFocus::Poem => {
                self.mode = self.mode.toggle();
                return PoemCreateAction::None;
            }
            _ => {}
        }
        match self.focus {
            PoemFocus::Poem => { self.poem_area.input(key); }
            PoemFocus::Author => { self.author_area.input(key); }
            PoemFocus::Tags => { self.tags_area.input(key); }
        }
        PoemCreateAction::None
    }

    pub fn handle_paste(&mut self, text: String) {
        let area = match self.focus {
            PoemFocus::Poem => &mut self.poem_area,
            PoemFocus::Author => &mut self.author_area,
            PoemFocus::Tags => &mut self.tags_area,
        };
        area.set_yank_text(&text);
        area.paste();
    }

    fn build_cards(&self) -> PoemCreateAction {
        let raw = self.poem_area.lines().join("\n");
        let raw = raw.trim().to_string();
        if raw.is_empty() {
            return PoemCreateAction::Cancel;
        }

        let author = self.author_area.lines().join("").trim().to_string();
        let tag_text = self.tags_area.lines().join(" ");
        let mut tags: Vec<String> = tag_text
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();
        if !author.is_empty() {
            // Prepend author as a tag (slugified: spaces → underscores)
            let author_tag = author.replace(' ', "_");
            tags.insert(0, format!("author:{author_tag}"));
        }

        let cards: Vec<NewCard> = poem_to_lpcg(&raw, self.mode)
            .into_iter()
            .map(|text| NewCard {
                text,
                back: String::new(),
                tags: tags.clone(),
                deck_id: self.deck_id,
                notetype_id: self.notetype_id,
            })
            .collect();
        if cards.is_empty() {
            return PoemCreateAction::Cancel;
        }
        PoemCreateAction::Save(cards)
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),  // hint bar
                Constraint::Min(6),     // poem textarea
                Constraint::Length(1),  // live preview
                Constraint::Length(3),  // author
                Constraint::Length(3),  // tags
                Constraint::Length(2),  // footer
            ])
            .split(area);

        let hint = Paragraph::new("Poem → Anki · Ctrl+Enter save · Tab cycle fields · g mode · Esc cancel")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(hint, chunks[0]);

        let focused_style = Style::default().fg(Color::Yellow);
        let unfocused_style = Style::default();

        self.poem_area.set_block(
            Block::default().borders(Borders::ALL).title(" Poem text ")
                .border_style(if self.focus == PoemFocus::Poem { focused_style } else { unfocused_style }),
        );
        self.author_area.set_block(
            Block::default().borders(Borders::ALL).title(" Author ")
                .border_style(if self.focus == PoemFocus::Author { focused_style } else { unfocused_style }),
        );
        self.tags_area.set_block(
            Block::default().borders(Borders::ALL).title(" Tags ")
                .border_style(if self.focus == PoemFocus::Tags { focused_style } else { unfocused_style }),
        );

        frame.render_widget(&self.poem_area, chunks[1]);

        // Live preview
        let raw = self.poem_area.lines().join("\n");
        let count = count_cards(raw.trim(), self.mode);
        let preview_style = if count > 0 {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let preview = Paragraph::new(format!(
            " → {} cards  [{}]",
            count,
            self.mode.label(),
        ))
        .style(preview_style);
        frame.render_widget(preview, chunks[2]);

        frame.render_widget(&self.author_area, chunks[3]);
        frame.render_widget(&self.tags_area, chunks[4]);

        let footer = Paragraph::new(Line::from(vec![
            Span::styled("[Ctrl+Enter]", Style::default().fg(Color::Yellow)),
            Span::raw(" Save  "),
            Span::styled("[Tab]", Style::default().fg(Color::Cyan)),
            Span::raw(" Cycle fields  "),
            Span::styled("[g]", Style::default().fg(Color::Green)),
            Span::raw(" Toggle mode  "),
            Span::styled("[Esc]", Style::default().fg(Color::DarkGray)),
            Span::raw(" Cancel"),
        ]));
        frame.render_widget(footer, chunks[5]);
    }
}
