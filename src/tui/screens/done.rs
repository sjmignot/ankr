use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use crossterm::event::{KeyCode, KeyEvent};
use crate::models::SessionStats;

pub struct DoneScreen {
    pub stats: SessionStats,
}

impl DoneScreen {
    pub fn new(stats: SessionStats) -> Self { Self { stats } }

    pub fn handle_key(&self, key: &KeyEvent) -> bool {
        matches!(key.code, KeyCode::Char('q') | KeyCode::Enter | KeyCode::Esc)
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let s = &self.stats;
        let lines = vec![
            Line::from(Span::styled(
                "Session complete!",
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(format!("  Reviewed:  {}", s.reviewed)),
            Line::from(format!("  New cards: {}", s.new_introduced)),
            Line::from(""),
            Line::from(vec![
                Span::styled("  Again: ", Style::default().fg(Color::Red)),
                Span::raw(format!("{}", s.again)),
                Span::styled("  Hard: ", Style::default().fg(Color::Yellow)),
                Span::raw(format!("{}", s.hard)),
                Span::styled("  Good: ", Style::default().fg(Color::Green)),
                Span::raw(format!("{}", s.good)),
                Span::styled("  Easy: ", Style::default().fg(Color::Cyan)),
                Span::raw(format!("{}", s.easy)),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "  [Enter / q] Back to deck select",
                Style::default().fg(Color::DarkGray),
            )),
        ];

        let para = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(" Done "));
        frame.render_widget(para, area);
    }
}
