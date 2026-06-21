use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use std::time::Duration;
use crate::models::Rating;

#[derive(Debug, Clone)]
pub enum AppEvent {
    Key(KeyEvent),
    Tick,
    AiResult(Result<Vec<crate::models::NewCard>, String>),
}

pub fn poll_event(timeout_ms: u64) -> Option<AppEvent> {
    if event::poll(Duration::from_millis(timeout_ms)).unwrap_or(false) {
        match event::read().ok()? {
            Event::Key(k) => Some(AppEvent::Key(k)),
            _ => None,
        }
    } else {
        Some(AppEvent::Tick)
    }
}

pub fn key_to_rating(key: &KeyEvent) -> Option<Rating> {
    match key.code {
        KeyCode::Char('1') => Some(Rating::Again),
        KeyCode::Char('2') => Some(Rating::Hard),
        KeyCode::Char('3') => Some(Rating::Good),
        KeyCode::Char('4') => Some(Rating::Easy),
        _ => None,
    }
}

pub fn is_quit(key: &KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('q'))
        || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
}

pub fn is_back(key: &KeyEvent) -> bool {
    matches!(key.code, KeyCode::Esc | KeyCode::Char('b'))
}
