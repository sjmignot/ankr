use std::collections::HashSet;
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
    decks: Vec<(Deck, u32, u32, u32)>,
    expanded: HashSet<String>,
    has_children: HashSet<String>,
    pub list_state: ListState,
}

impl DeckSelectScreen {
    pub fn new(decks: Vec<(Deck, u32, u32, u32)>) -> Self {
        let has_children: HashSet<String> = decks.iter()
            .flat_map(|(d, ..)| {
                let parts: Vec<&str> = d.name.split("::").collect();
                (1..parts.len()).map(move |i| parts[..i].join("::"))
            })
            .collect();

        let mut list_state = ListState::default();
        if decks.iter().any(|(d, ..)| !d.name.contains("::")) {
            list_state.select(Some(0));
        } else if !decks.is_empty() {
            list_state.select(Some(0));
        }

        Self { decks, expanded: HashSet::new(), has_children, list_state }
    }

    fn visible_indices(&self) -> Vec<usize> {
        self.decks.iter().enumerate()
            .filter_map(|(i, (deck, ..))| {
                let parts: Vec<&str> = deck.name.split("::").collect();
                for depth in 1..parts.len() {
                    let ancestor = parts[..depth].join("::");
                    if !self.expanded.contains(&ancestor) {
                        return None;
                    }
                }
                Some(i)
            })
            .collect()
    }

    fn aggregated_counts(&self, name: &str) -> (u32, u32, u32) {
        let prefix = format!("{}::", name);
        self.decks.iter()
            .filter(|(d, ..)| d.name == name || d.name.starts_with(&prefix))
            .fold((0, 0, 0), |(n, l, r), (_, new, lrn, rev)| (n + new, l + lrn, r + rev))
    }

    pub fn selected_deck(&self) -> Option<&Deck> {
        let vis = self.visible_indices();
        self.list_state.selected()
            .and_then(|i| vis.get(i).copied())
            .and_then(|idx| self.decks.get(idx).map(|(d, ..)| d))
    }

    fn selected_name(&self) -> Option<String> {
        let vis = self.visible_indices();
        self.list_state.selected()
            .and_then(|i| vis.get(i).copied())
            .and_then(|idx| self.decks.get(idx).map(|(d, ..)| d.name.clone()))
    }

    pub fn handle_key(&mut self, key: &KeyEvent) -> DeckAction {
        let n = self.visible_indices().len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                let i = self.list_state.selected().unwrap_or(0);
                if i > 0 { self.list_state.select(Some(i - 1)); }
                DeckAction::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let i = self.list_state.selected().unwrap_or(0);
                if i + 1 < n { self.list_state.select(Some(i + 1)); }
                DeckAction::None
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if let Some(name) = self.selected_name() {
                    if self.has_children.contains(&name) && !self.expanded.contains(&name) {
                        self.expanded.insert(name);
                    }
                }
                DeckAction::None
            }
            KeyCode::Left | KeyCode::Char('h') => {
                if let Some(name) = self.selected_name() {
                    let parts: Vec<&str> = name.split("::").collect();
                    let target = if parts.len() > 1 {
                        // On a child: collapse its immediate parent
                        parts[..parts.len()-1].join("::")
                    } else {
                        name.clone()
                    };
                    if self.expanded.remove(&target) {
                        // Navigate to the newly-collapsed entry
                        let new_vis = self.visible_indices();
                        if let Some(new_idx) = new_vis.iter().position(|&idx| {
                            self.decks.get(idx).map(|(d, ..)| d.name == target).unwrap_or(false)
                        }) {
                            self.list_state.select(Some(new_idx));
                        }
                    }
                }
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
            .constraints([Constraint::Length(3), Constraint::Min(1), Constraint::Length(1)])
            .split(area);

        let title = Paragraph::new("ankr — select a deck")
            .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
            .block(Block::default().borders(Borders::BOTTOM));
        frame.render_widget(title, chunks[0]);

        let vis = self.visible_indices();
        let items: Vec<ListItem> = vis.iter().map(|&idx| {
            let (deck, own_new, own_lrn, own_rev) = &self.decks[idx];
            let depth = deck.name.matches("::").count();
            let short = deck.name.rsplit("::").next().unwrap_or(&deck.name);

            let is_expandable = self.has_children.contains(&deck.name);
            let is_expanded = self.expanded.contains(&deck.name);

            let (indicator, new, lrn, rev) = if is_expandable && !is_expanded {
                let (n, l, r) = self.aggregated_counts(&deck.name);
                ("▶ ", n, l, r)
            } else if is_expandable {
                ("▼ ", *own_new, *own_lrn, *own_rev)
            } else {
                ("  ", *own_new, *own_lrn, *own_rev)
            };

            let label = format!("{}{}{}", "  ".repeat(depth), indicator, short);
            let line = Line::from(vec![
                Span::raw(format!("{:<42}", label)),
                Span::styled(format!(" {new}n "), Style::default().fg(Color::Cyan)),
                Span::styled(format!("{lrn}l "), Style::default().fg(Color::Red)),
                Span::styled(format!("{rev}r"), Style::default().fg(Color::Green)),
            ]);
            ListItem::new(line)
        }).collect();

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(" Decks "))
            .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
            .highlight_symbol("» ");

        frame.render_stateful_widget(list, chunks[1], &mut self.list_state);

        let footer = Line::from(vec![
            Span::styled("[Enter]", Style::default().fg(Color::Yellow)),
            Span::raw(" Review  "),
            Span::styled("[a]", Style::default().fg(Color::Cyan)),
            Span::raw(" AI  "),
            Span::styled("[p]", Style::default().fg(Color::Cyan)),
            Span::raw(" Poem  "),
            Span::styled("[l/h]", Style::default().fg(Color::DarkGray)),
            Span::raw(" Expand/Collapse  "),
            Span::styled("[q]", Style::default().fg(Color::DarkGray)),
            Span::raw(" Quit"),
        ]);
        frame.render_widget(Paragraph::new(footer), chunks[2]);
    }
}

pub enum DeckAction {
    None,
    Select,
    Quit,
}
