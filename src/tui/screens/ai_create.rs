use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tui_textarea::TextArea;
use crate::models::NewCard;

pub enum AiAction {
    None,
    Generate(String),
    AcceptCard(NewCard),
    SkipCard,
    Cancel,
}

pub enum AiState {
    Editing,
    Loading(u8),
    Reviewing(Vec<NewCard>, usize),
    Done,
}

pub struct AiCreateScreen {
    textarea: TextArea<'static>,
    pub state: AiState,
    pub deck_id: i64,
    pub notetype_id: i64,
    pub accepted: Vec<NewCard>,
}

impl AiCreateScreen {
    pub fn new(deck_id: i64, notetype_id: i64) -> Self {
        let mut textarea = TextArea::default();
        textarea.set_block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Paste text — Claude will generate cloze cards "),
        );
        Self {
            textarea,
            state: AiState::Editing,
            deck_id,
            notetype_id,
            accepted: Vec::new(),
        }
    }

    pub fn set_cards(&mut self, cards: Vec<NewCard>) {
        if cards.is_empty() {
            self.state = AiState::Done;
        } else {
            self.state = AiState::Reviewing(cards, 0);
        }
    }

    pub fn set_error(&mut self, msg: String) {
        self.state = AiState::Editing;
        let mut ta = TextArea::default();
        ta.set_block(Block::default().borders(Borders::ALL).title(format!(" Error: {msg} ")));
        self.textarea = ta;
    }

    pub fn tick(&mut self) {
        if let AiState::Loading(ref mut f) = self.state {
            *f = (*f + 1) % 8;
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> AiAction {
        match &self.state {
            AiState::Editing => match key.code {
                KeyCode::Esc => return AiAction::Cancel,
                KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let text = self.textarea.lines().join("\n").trim().to_string();
                    if !text.is_empty() {
                        self.state = AiState::Loading(0);
                        return AiAction::Generate(text);
                    }
                }
                _ => { self.textarea.input(key); }
            },
            AiState::Loading(_) => {
                if key.code == KeyCode::Esc { return AiAction::Cancel; }
            }
            AiState::Reviewing(cards, idx) => {
                let idx = *idx;
                let total = cards.len();
                match key.code {
                    KeyCode::Char('a') | KeyCode::Enter => {
                        let card = cards[idx].clone();
                        self.accepted.push(card.clone());
                        self.advance_review(idx, total);
                        return AiAction::AcceptCard(card);
                    }
                    KeyCode::Char('s') | KeyCode::Char('n') => {
                        self.advance_review(idx, total);
                        return AiAction::SkipCard;
                    }
                    KeyCode::Esc => return AiAction::Cancel,
                    _ => {}
                }
            }
            AiState::Done => return AiAction::Cancel,
        }
        AiAction::None
    }

    fn advance_review(&mut self, current: usize, total: usize) {
        if current + 1 >= total {
            self.state = AiState::Done;
        } else if let AiState::Reviewing(_, idx) = &mut self.state {
            *idx = current + 1;
        }
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        match &self.state {
            AiState::Editing => self.render_editing(frame, area),
            AiState::Loading(f) => self.render_loading(frame, area, *f),
            AiState::Reviewing(cards, idx) => {
                let idx = *idx;
                let total = cards.len();
                let card = cards[idx].clone();
                self.render_review(frame, area, &card, idx, total);
            }
            AiState::Done => self.render_done(frame, area),
        }
    }

    fn render_editing(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Min(6), Constraint::Length(2)])
            .split(area);

        frame.render_widget(
            Paragraph::new("AI card creation — Ctrl+Enter to generate · Esc to cancel")
                .style(Style::default().fg(Color::DarkGray)),
            chunks[0],
        );
        frame.render_widget(&self.textarea, chunks[1]);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("[Ctrl+Enter]", Style::default().fg(Color::Yellow)),
                Span::raw(" Generate cards with Claude"),
            ])),
            chunks[2],
        );
    }

    fn render_loading(&self, frame: &mut Frame, area: Rect, frame_idx: u8) {
        let spinner = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];
        let para = Paragraph::new(format!(
            "\n\n  {} Asking Claude to generate cards...",
            spinner[frame_idx as usize % spinner.len()]
        ))
        .style(Style::default().fg(Color::Yellow))
        .block(Block::default().borders(Borders::ALL));
        frame.render_widget(para, area);
    }

    fn render_review(&self, frame: &mut Frame, area: Rect, card: &NewCard, idx: usize, total: usize) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Min(4), Constraint::Length(3)])
            .split(area);

        frame.render_widget(
            Paragraph::new(format!("Card {} of {} — review generated cards", idx + 1, total))
                .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            chunks[0],
        );

        let card_text = Paragraph::new(card.text.clone())
            .block(Block::default().borders(Borders::ALL).title(" Preview "))
            .wrap(Wrap { trim: false });
        frame.render_widget(card_text, chunks[1]);

        let footer = Paragraph::new(Line::from(vec![
            Span::styled("[a/Enter]", Style::default().fg(Color::Green)),
            Span::raw(" Accept  "),
            Span::styled("[s/n]", Style::default().fg(Color::Yellow)),
            Span::raw(" Skip  "),
            Span::styled("[Esc]", Style::default().fg(Color::DarkGray)),
            Span::raw(" Cancel"),
        ]));
        frame.render_widget(footer, chunks[2]);
    }

    fn render_done(&self, frame: &mut Frame, area: Rect) {
        let para = Paragraph::new(format!(
            "\n\n  Saved {} card(s). Returning...",
            self.accepted.len()
        ))
        .style(Style::default().fg(Color::Green))
        .block(Block::default().borders(Borders::ALL));
        frame.render_widget(para, area);
    }
}
