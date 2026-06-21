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
use crate::render::poem::{GranularityMode, count_cloze_units, poem_to_cloze};

pub enum PoemCreateAction {
    None,
    Save(NewCard),
    Cancel,
}

pub struct PoemCreateScreen {
    poem_area: TextArea<'static>,
    tags_area: TextArea<'static>,
    focus_tags: bool,
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

        let mut tags_area = TextArea::default();
        tags_area.set_block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Title / tags (space-separated) "),
        );
        tags_area.set_placeholder_text("shakespeare sonnets poem");

        Self {
            poem_area,
            tags_area,
            focus_tags: false,
            mode: GranularityMode::Line,
            deck_id,
            notetype_id,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> PoemCreateAction {
        match key.code {
            KeyCode::Esc => return PoemCreateAction::Cancel,
            KeyCode::Tab => {
                self.focus_tags = !self.focus_tags;
                return PoemCreateAction::None;
            }
            // Ctrl+Enter saves from anywhere
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return self.build_card();
            }
            // Plain Enter in tags field saves
            KeyCode::Enter if self.focus_tags => {
                return self.build_card();
            }
            // [g] toggles granularity mode when not in tags
            KeyCode::Char('g') if !self.focus_tags => {
                self.mode = self.mode.toggle();
                return PoemCreateAction::None;
            }
            _ => {}
        }
        if self.focus_tags {
            self.tags_area.input(key);
        } else {
            self.poem_area.input(key);
        }
        PoemCreateAction::None
    }

    fn build_card(&self) -> PoemCreateAction {
        let raw = self.poem_area.lines().join("\n");
        let raw = raw.trim();
        if raw.is_empty() {
            return PoemCreateAction::Cancel;
        }
        let text = poem_to_cloze(raw, self.mode);
        let tags: Vec<String> = self.tags_area.lines()
            .join(" ")
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();
        PoemCreateAction::Save(NewCard {
            text,
            back: String::new(),
            tags,
            deck_id: self.deck_id,
            notetype_id: self.notetype_id,
        })
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),  // hint bar
                Constraint::Min(8),     // poem textarea
                Constraint::Length(1),  // live preview
                Constraint::Length(3),  // tags textarea
                Constraint::Length(2),  // footer
            ])
            .split(area);

        let hint = Paragraph::new("Poem → Anki · Enter/Ctrl+Enter to save · Tab switch · g mode · Esc cancel")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(hint, chunks[0]);

        // Highlight focused area border
        let poem_style = if !self.focus_tags {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default()
        };
        let tags_style = if self.focus_tags {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default()
        };
        self.poem_area.set_block(
            Block::default().borders(Borders::ALL).title(" Poem text ").border_style(poem_style),
        );
        self.tags_area.set_block(
            Block::default().borders(Borders::ALL).title(" Title / tags ").border_style(tags_style),
        );

        frame.render_widget(&self.poem_area, chunks[1]);

        // Live preview
        let raw = self.poem_area.lines().join("\n");
        let count = count_cloze_units(raw.trim(), self.mode);
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

        frame.render_widget(&self.tags_area, chunks[3]);

        let footer = Paragraph::new(Line::from(vec![
            Span::styled("[Enter]", Style::default().fg(Color::Yellow)),
            Span::raw(" Save  "),
            Span::styled("[Tab]", Style::default().fg(Color::Cyan)),
            Span::raw(" Switch field  "),
            Span::styled("[g]", Style::default().fg(Color::Green)),
            Span::raw(" Toggle mode  "),
            Span::styled("[Esc]", Style::default().fg(Color::DarkGray)),
            Span::raw(" Cancel"),
        ]));
        frame.render_widget(footer, chunks[4]);
    }
}
