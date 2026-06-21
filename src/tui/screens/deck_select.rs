use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};
use crossterm::event::{KeyCode, KeyEvent};
use crate::models::Deck;

pub struct DeckSelectScreen {
    pub decks: Vec<(Deck, u32, u32, u32)>, // (deck, new, learning, review)
    pub list_state: ListState,
}

impl DeckSelectScreen {
    pub fn new(decks: Vec<(Deck, u32, u32, u32)>) -> Self {
        let mut list_state = ListState::default();
        if !decks.is_empty() { list_state.select(Some(0)); }
        Self { decks, list_state }
    }

    pub fn selected_deck(&self) -> Option<&Deck> {
        self.list_state.selected().and_then(|i| self.decks.get(i).map(|(d, ..)| d))
    }

    pub fn handle_key(&mut self, key: &KeyEvent) -> DeckAction {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                let i = self.list_state.selected().unwrap_or(0);
                if i > 0 { self.list_state.select(Some(i - 1)); }
                DeckAction::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let i = self.list_state.selected().unwrap_or(0);
                if i + 1 < self.decks.len() { self.list_state.select(Some(i + 1)); }
                DeckAction::None
            }
            KeyCode::Enter | KeyCode::Char(' ') => DeckAction::Select,
            KeyCode::Char('q') => DeckAction::Quit,
            _ => DeckAction::None,
        }
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(1)])
            .split(area);

        let title = Paragraph::new("ankr — select a deck")
            .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
            .block(Block::default().borders(Borders::BOTTOM));
        frame.render_widget(title, chunks[0]);

        let items: Vec<ListItem> = self.decks.iter().map(|(deck, new, learning, review)| {
            let total = new + learning + review;
            let count_style = if total > 0 {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let line = Line::from(vec![
                Span::raw(format!("{:<40}", deck.name)),
                Span::styled(format!(" {new}n "), Style::default().fg(Color::Cyan)),
                Span::styled(format!("{learning}l "), Style::default().fg(Color::Red)),
                Span::styled(format!("{review}r", ), Style::default().fg(Color::Green)),
            ]);
            ListItem::new(line)
        }).collect();

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(" Decks "))
            .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
            .highlight_symbol("▶ ");

        frame.render_stateful_widget(list, chunks[1], &mut self.list_state);
    }
}

pub enum DeckAction {
    None,
    Select,
    Quit,
}
