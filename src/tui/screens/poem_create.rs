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
    Save { cards: Vec<NewCard>, subdeck_path: String },
    Cancel,
}

#[derive(Clone, Copy, PartialEq)]
enum PoemFocus { Poem, Title, Author, Tags }

pub struct PoemCreateScreen {
    poem_area: TextArea<'static>,
    title_area: TextArea<'static>,
    author_area: TextArea<'static>,
    tags_area: TextArea<'static>,
    focus: PoemFocus,
    mode: GranularityMode,
    deck_id: i64,
    parent_deck_name: String,
    notetype_id: i64,
}

impl PoemCreateScreen {
    pub fn new(parent_deck_name: String, deck_id: i64, notetype_id: i64) -> Self {
        let mut poem_area = TextArea::default();
        poem_area.set_block(Block::default().borders(Borders::ALL).title(" Poem text "));
        poem_area.set_placeholder_text("Shall I compare thee to a summer's day?");

        let mut title_area = TextArea::default();
        title_area.set_block(Block::default().borders(Borders::ALL).title(" Title "));
        title_area.set_placeholder_text("Sonnet 18");

        let mut author_area = TextArea::default();
        author_area.set_block(Block::default().borders(Borders::ALL).title(" Author "));
        author_area.set_placeholder_text("Shakespeare");

        let mut tags_area = TextArea::default();
        tags_area.set_block(Block::default().borders(Borders::ALL).title(" Tags (space-separated) "));
        tags_area.set_placeholder_text("sonnets poem");

        Self {
            poem_area,
            title_area,
            author_area,
            tags_area,
            focus: PoemFocus::Poem,
            mode: GranularityMode::Line,
            deck_id,
            parent_deck_name,
            notetype_id,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> PoemCreateAction {
        match key.code {
            KeyCode::Esc => return PoemCreateAction::Cancel,
            KeyCode::Tab => {
                self.focus = match self.focus {
                    PoemFocus::Poem => PoemFocus::Title,
                    PoemFocus::Title => PoemFocus::Author,
                    PoemFocus::Author => PoemFocus::Tags,
                    PoemFocus::Tags => PoemFocus::Poem,
                };
                return PoemCreateAction::None;
            }
            KeyCode::BackTab => {
                self.focus = match self.focus {
                    PoemFocus::Poem => PoemFocus::Tags,
                    PoemFocus::Tags => PoemFocus::Author,
                    PoemFocus::Author => PoemFocus::Title,
                    PoemFocus::Title => PoemFocus::Poem,
                };
                return PoemCreateAction::None;
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return self.build_cards();
            }
            KeyCode::Enter if self.focus != PoemFocus::Poem => {
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
            PoemFocus::Title => { self.title_area.input(key); }
            PoemFocus::Author => { self.author_area.input(key); }
            PoemFocus::Tags => { self.tags_area.input(key); }
        }
        PoemCreateAction::None
    }

    pub fn handle_paste(&mut self, text: String) {
        let area = match self.focus {
            PoemFocus::Poem => &mut self.poem_area,
            PoemFocus::Title => &mut self.title_area,
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

        let title = self.title_area.lines().join("").trim().to_string();
        let author = self.author_area.lines().join("").trim().to_string();
        let tag_text = self.tags_area.lines().join(" ");
        let mut tags: Vec<String> = tag_text
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();
        if !author.is_empty() {
            tags.insert(0, format!("author:{}", author.replace(' ', "_")));
        }
        if !title.is_empty() {
            tags.insert(0, format!("title:{}", title.replace(' ', "_")));
        }

        // Build subdeck path: Parent > Author > Title (omit empty components).
        let mut path = self.parent_deck_name.clone();
        if !author.is_empty() { path = format!("{}::{}", path, author); }
        if !title.is_empty()  { path = format!("{}::{}", path, title); }

        let cards: Vec<NewCard> = poem_to_lpcg(&raw, self.mode)
            .into_iter()
            .map(|text| NewCard {
                text,
                back: String::new(),
                tags: tags.clone(),
                deck_id: self.deck_id,   // placeholder; caller remaps via subdeck_path
                notetype_id: self.notetype_id,
            })
            .collect();
        if cards.is_empty() {
            return PoemCreateAction::Cancel;
        }
        PoemCreateAction::Save { cards, subdeck_path: path }
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),  // hint bar
                Constraint::Min(5),     // poem textarea
                Constraint::Length(1),  // live preview
                Constraint::Length(3),  // title
                Constraint::Length(3),  // author
                Constraint::Length(3),  // tags
                Constraint::Length(2),  // footer
            ])
            .split(area);

        let hint = Paragraph::new("Poem → Anki · Ctrl+Enter save · Tab/S-Tab cycle fields · g mode · Esc cancel")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(hint, chunks[0]);

        let focused_style = Style::default().fg(Color::Yellow);
        let unfocused_style = Style::default();

        self.poem_area.set_block(
            Block::default().borders(Borders::ALL).title(" Poem text ")
                .border_style(if self.focus == PoemFocus::Poem { focused_style } else { unfocused_style }),
        );
        self.title_area.set_block(
            Block::default().borders(Borders::ALL).title(" Title ")
                .border_style(if self.focus == PoemFocus::Title { focused_style } else { unfocused_style }),
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

        let raw = self.poem_area.lines().join("\n");
        let count = count_cards(raw.trim(), self.mode);
        let preview_style = if count > 0 {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        frame.render_widget(
            Paragraph::new(format!(" → {} cards  [{}]", count, self.mode.label())).style(preview_style),
            chunks[2],
        );

        frame.render_widget(&self.title_area, chunks[3]);
        frame.render_widget(&self.author_area, chunks[4]);
        frame.render_widget(&self.tags_area, chunks[5]);

        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("[Ctrl+Enter]", Style::default().fg(Color::Yellow)),
                Span::raw(" Save  "),
                Span::styled("[Tab/S-Tab]", Style::default().fg(Color::Cyan)),
                Span::raw(" Cycle fields  "),
                Span::styled("[g]", Style::default().fg(Color::Green)),
                Span::raw(" Toggle mode  "),
                Span::styled("[Esc]", Style::default().fg(Color::DarkGray)),
                Span::raw(" Cancel"),
            ])),
            chunks[6],
        );
    }
}
