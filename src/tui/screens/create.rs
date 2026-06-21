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

pub enum CreateAction {
    None,
    Save(NewCard),
    Cancel,
}

pub struct CreateScreen {
    textarea: TextArea<'static>,
    tags_area: TextArea<'static>,
    focus_tags: bool,
    deck_id: i64,
    notetype_id: i64,
}

impl CreateScreen {
    pub fn new(deck_id: i64, notetype_id: i64) -> Self {
        let mut textarea = TextArea::default();
        textarea.set_block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Card text (use {{c1::answer}} for cloze) "),
        );
        textarea.set_placeholder_text("The {{c1::capital of France}} is Paris.");

        let mut tags_area = TextArea::default();
        tags_area.set_block(Block::default().borders(Borders::ALL).title(" Tags (space-separated) "));
        tags_area.set_placeholder_text("geography europe");

        Self {
            textarea,
            tags_area,
            focus_tags: false,
            deck_id,
            notetype_id,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> CreateAction {
        match key.code {
            KeyCode::Esc => return CreateAction::Cancel,
            KeyCode::Tab => {
                self.focus_tags = !self.focus_tags;
                return CreateAction::None;
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return self.build_card();
            }
            _ => {}
        }
        if self.focus_tags {
            self.tags_area.input(key);
        } else {
            self.textarea.input(key);
        }
        CreateAction::None
    }

    fn build_card(&self) -> CreateAction {
        let text = self.textarea.lines().join("\n").trim().to_string();
        if text.is_empty() { return CreateAction::Cancel; }
        let tags: Vec<String> = self.tags_area.lines()
            .join(" ")
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();
        CreateAction::Save(NewCard {
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
                Constraint::Length(2),
                Constraint::Min(6),
                Constraint::Length(3),
                Constraint::Length(2),
            ])
            .split(area);

        let hint = Paragraph::new("Create card — Ctrl+Enter to save · Tab to switch · Esc to cancel")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(hint, chunks[0]);

        frame.render_widget(&self.textarea, chunks[1]);
        frame.render_widget(&self.tags_area, chunks[2]);

        let footer = Paragraph::new(Line::from(vec![
            Span::styled("[Ctrl+Enter]", Style::default().fg(Color::Yellow)),
            Span::raw(" Save  "),
            Span::styled("[Tab]", Style::default().fg(Color::Cyan)),
            Span::raw(" Switch field  "),
            Span::styled("[Esc]", Style::default().fg(Color::DarkGray)),
            Span::raw(" Cancel"),
        ]));
        frame.render_widget(footer, chunks[3]);
    }
}
