pub mod events;
pub mod screens;

use std::io;
use std::path::PathBuf;
use std::time::Duration;
use crossterm::{
    event::{
        DisableBracketedPaste, DisableMouseCapture,
        EnableBracketedPaste, EnableMouseCapture, KeyCode,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, enable_raw_mode, disable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::sync::mpsc;

use crate::db::{DbConn, queries};
use crate::error::Result;
use crate::models::*;
use crate::scheduler::FsrsScheduler;
use crate::review::ReviewQueue;
use crate::ai::ClaudeClient;
use screens::{AiCreateScreen, CreateScreen, DeckSelectScreen, DoneScreen, PoemCreateScreen, ReviewScreen};
use screens::ai_create::{AiAction, AiState};
use screens::deck_select::DeckAction;
use screens::review::ReviewAction;
use screens::create::CreateAction;
use screens::poem_create::PoemCreateAction;

pub struct AppConfig {
    pub db_path: PathBuf,
    pub media_dir: PathBuf,
    pub new_limit: u32,
    pub review_limit: u32,
    pub readonly: bool,
}

enum Screen {
    DeckSelect(DeckSelectScreen),
    Review {
        screen: ReviewScreen,
        queue: ReviewQueue,
        scheduler: FsrsScheduler,
        crt: i64,
        deck_id: i64,
        deck_name: String,
        notetype_id: i64,
    },
    Done(DoneScreen),
    Create(CreateScreen),
    AiCreate(AiCreateScreen),
    PoemCreate(PoemCreateScreen),
}

/// Run just the poem creation screen, then exit. Used by `ankr poem` with no input.
pub async fn run_poem(db: DbConn, deck_name: String, deck_id: i64, notetype_id: i64) -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture, EnableBracketedPaste)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_poem_app(&mut terminal, &db, deck_name, deck_id, notetype_id).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture, DisableBracketedPaste)?;
    terminal.show_cursor()?;
    result
}

async fn run_poem_app(
    terminal: &mut ratatui::Terminal<CrosstermBackend<io::Stdout>>,
    db: &DbConn,
    deck_name: String,
    deck_id: i64,
    notetype_id: i64,
) -> anyhow::Result<()> {
    let mut screen = PoemCreateScreen::new(deck_name, deck_id, notetype_id);
    loop {
        terminal.draw(|f| screen.render(f, f.area()))?;

        let maybe_event = tokio::task::spawn_blocking(|| {
            if crossterm::event::poll(Duration::from_millis(100)).unwrap_or(false) {
                crossterm::event::read().ok()
            } else {
                None
            }
        })
        .await?;

        let Some(event) = maybe_event else { continue };

        let action = match event {
            crossterm::event::Event::Paste(text) => {
                screen.handle_paste(text);
                PoemCreateAction::None
            }
            crossterm::event::Event::Key(key) => screen.handle_key(key),
            _ => PoemCreateAction::None,
        };

        match action {
            PoemCreateAction::None => {}
            PoemCreateAction::Cancel => break,
            PoemCreateAction::Save { cards, subdeck_path } => {
                let did = queries::get_or_create_deck_path(&db.conn, &subdeck_path)?;
                let n = cards.len();
                for mut card in cards {
                    card.deck_id = did;
                    queries::insert_note(&db.conn, &card)?;
                }
                drop(screen);
                eprintln!("Created {n} cards in \"{subdeck_path}\".");
                break;
            }
        }
    }
    Ok(())
}

pub async fn run(db: DbConn, config: AppConfig) -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture, EnableBracketedPaste)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, db, config).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture, DisableBracketedPaste)?;
    terminal.show_cursor()?;

    result
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    db: DbConn,
    config: AppConfig,
) -> anyhow::Result<()> {
    let crt = queries::get_collection_crt(&db.conn)?;

    let (ai_tx, mut ai_rx) = mpsc::channel::<std::result::Result<Vec<NewCard>, String>>(4);

    let mut screen = Screen::DeckSelect(build_deck_select(&db, crt, config.review_limit)?);

    loop {
        // Drain AI results
        while let Ok(result) = ai_rx.try_recv() {
            if let Screen::AiCreate(ref mut ai_screen) = screen {
                match result {
                    Ok(cards) => ai_screen.set_cards(cards),
                    Err(e) => ai_screen.set_error(e),
                }
            }
        }
        if let Screen::AiCreate(ref mut s) = screen { s.tick(); }

        // Render
        terminal.draw(|f| {
            let area = f.area();
            match &mut screen {
                Screen::DeckSelect(s) => s.render(f, area),
                Screen::Review { screen: s, .. } => s.render(f, area),
                Screen::Done(s) => s.render(f, area),
                Screen::Create(s) => s.render(f, area),
                Screen::AiCreate(s) => s.render(f, area),
                Screen::PoemCreate(s) => s.render(f, area),
            }
        })?;

        // Poll event (100ms so spinner animates)
        let maybe_event = tokio::task::spawn_blocking(|| {
            if crossterm::event::poll(Duration::from_millis(100)).unwrap_or(false) {
                crossterm::event::read().ok()
            } else {
                None
            }
        }).await?;

        let Some(event) = maybe_event else { continue };

        // Handle paste for the poem screen before key dispatch
        if let crossterm::event::Event::Paste(ref text) = event {
            if let Screen::PoemCreate(ref mut s) = screen {
                s.handle_paste(text.clone());
            }
            continue;
        }

        // All other screens only use key events
        let key = match event {
            crossterm::event::Event::Key(k) => k,
            _ => continue,
        };

        match &mut screen {
            // ── Deck Select ──────────────────────────────────────────────────
            Screen::DeckSelect(s) => {
                if key.code == KeyCode::Char('p') {
                    if let Some(deck) = s.selected_deck() {
                        let cloze_id = queries::get_cloze_notetype_id(&db.conn)?.unwrap_or(0);
                        screen = Screen::PoemCreate(PoemCreateScreen::new(deck.name.clone(), deck.id, cloze_id));
                    }
                    continue;
                }
                match s.handle_key(&key) {
                    DeckAction::Quit => break,
                    DeckAction::None => {}
                    DeckAction::Select => {
                        if let Some(deck) = s.selected_deck() {
                            let deck_id = deck.id;
                            let deck_name = deck.name.clone();
                            let today = queries::today_day(crt);
                            let now = queries::now_unix();

                            let learning = queries::get_learning_cards(&db.conn, deck_id, now)?;
                            let due = queries::get_due_cards(&db.conn, deck_id, today, crt, config.review_limit)?;
                            let new = queries::get_new_cards(&db.conn, deck_id, config.new_limit as i64)?;

                            let notetypes = queries::get_all_notetypes(&db.conn)?;
                            let notetype_id = notetypes.first().map(|(id, _)| *id).unwrap_or(0);

                            let mut queue = ReviewQueue::new(learning, due, new, config.new_limit);

                            if queue.total_remaining() == 0 {
                                screen = Screen::Done(DoneScreen::new(SessionStats::default()));
                                continue;
                            }

                            if let Some(card) = queue.next() {
                                let note = queries::get_resolved_note(&db.conn, &card)?;
                                screen = Screen::Review {
                                    screen: ReviewScreen::new(card, note, config.media_dir.clone()),
                                    queue,
                                    scheduler: FsrsScheduler::new(0.9),
                                    crt,
                                    deck_id,
                                    deck_name,
                                    notetype_id,
                                };
                            }
                        }
                    }
                }
            }

            // ── Review ───────────────────────────────────────────────────────
            Screen::Review { screen: rev, queue, scheduler, crt: review_crt, deck_id, deck_name, notetype_id } => {
                // Only intercept [c]/[a] when not actively typing an answer.
                if !rev.is_typing() {
                    match key.code {
                        KeyCode::Char('c') => {
                            screen = Screen::Create(CreateScreen::new(*deck_id, *notetype_id));
                            continue;
                        }
                        KeyCode::Char('a') => {
                            screen = Screen::AiCreate(AiCreateScreen::new(*deck_id, *notetype_id));
                            continue;
                        }
                        KeyCode::Char('p') => {
                            let cloze_id = queries::get_cloze_notetype_id(&db.conn)?.unwrap_or(0);
                            screen = Screen::PoemCreate(PoemCreateScreen::new(deck_name.clone(), *deck_id, cloze_id));
                            continue;
                        }
                        _ => {}
                    }
                }

                match rev.handle_key(&key) {
                    ReviewAction::Quit => break,
                    ReviewAction::None => {}
                    ReviewAction::Back => {
                        screen = Screen::DeckSelect(build_deck_select(&db, crt, config.review_limit)?);
                    }
                    ReviewAction::Rated(rating, ms) => {
                        let was_new = rev.card.card_type == CardType::New;
                        let new_state = scheduler.schedule(&rev.card, rating, *review_crt);
                        let result = ReviewResult {
                            card_id: rev.card.id,
                            note_id: rev.note.id,
                            rating: rating as u8,
                            time_taken_ms: ms,
                            old_ivl: rev.card.ivl,
                            old_factor: rev.card.factor,
                            old_type: rev.card.card_type as i32,
                            new_state: new_state.into(),
                            reviewed_at_ms: queries::now_ms(),
                        };

                        if !config.readonly {
                            queries::write_review(&db.conn, &result, *review_crt)?;
                        }

                        queue.record(rating, was_new);

                        if rating == Rating::Again {
                            queue.requeue(rev.card.clone());
                        }

                        if let Some(next_card) = queue.next() {
                            let note = queries::get_resolved_note(&db.conn, &next_card)?;
                            *rev = ReviewScreen::new(next_card, note, config.media_dir.clone());
                        } else {
                            let stats = queue.stats.clone();
                            screen = Screen::Done(DoneScreen::new(stats));
                        }
                    }
                }
            }

            // ── Done ─────────────────────────────────────────────────────────
            Screen::Done(s) => {
                if s.handle_key(&key) {
                    screen = Screen::DeckSelect(build_deck_select(&db, crt, config.review_limit)?);
                }
            }

            // ── Manual Create ─────────────────────────────────────────────────
            Screen::Create(s) => match s.handle_key(key) {
                CreateAction::None => {}
                CreateAction::Cancel => {
                    screen = Screen::DeckSelect(build_deck_select(&db, crt, config.review_limit)?);
                }
                CreateAction::Save(card) => {
                    if !config.readonly {
                        queries::insert_note(&db.conn, &card)?;
                    }
                    screen = Screen::DeckSelect(build_deck_select(&db, crt, config.review_limit)?);
                }
            },

            // ── AI Create ─────────────────────────────────────────────────────
            Screen::AiCreate(s) => match s.handle_key(key) {
                AiAction::None => {}
                AiAction::Cancel => {
                    screen = Screen::DeckSelect(build_deck_select(&db, crt, config.review_limit)?);
                }
                AiAction::Generate(text) => {
                    let did = s.deck_id;
                    let ntid = s.notetype_id;
                    let tx = ai_tx.clone();
                    tokio::spawn(async move {
                        let result = match ClaudeClient::from_env() {
                            Ok(client) => client.generate_cards(&text, "", did, ntid).await
                                .map_err(|e| e.to_string()),
                            Err(e) => Err(e.to_string()),
                        };
                        let _ = tx.send(result).await;
                    });
                }
                AiAction::AcceptCard(card) => {
                    if !config.readonly {
                        let _ = queries::insert_note(&db.conn, &card);
                    }
                    if matches!(s.state, AiState::Done) {
                        screen = Screen::DeckSelect(build_deck_select(&db, crt, config.review_limit)?);
                    }
                }
                AiAction::SkipCard => {
                    if matches!(s.state, AiState::Done) {
                        screen = Screen::DeckSelect(build_deck_select(&db, crt, config.review_limit)?);
                    }
                }
            },

            // ── Poem Create ───────────────────────────────────────────────────
            Screen::PoemCreate(s) => match s.handle_key(key) {
                PoemCreateAction::None => {}
                PoemCreateAction::Cancel => {
                    screen = Screen::DeckSelect(build_deck_select(&db, crt, config.review_limit)?);
                }
                PoemCreateAction::Save { cards, subdeck_path } => {
                    if !config.readonly {
                        let did = queries::get_or_create_deck_path(&db.conn, &subdeck_path)?;
                        for mut card in cards {
                            card.deck_id = did;
                            queries::insert_note(&db.conn, &card)?;
                        }
                    }
                    screen = Screen::DeckSelect(build_deck_select(&db, crt, config.review_limit)?);
                }
            },
        }
    }

    Ok(())
}

fn build_deck_select(db: &DbConn, crt: i64, review_limit: u32) -> anyhow::Result<DeckSelectScreen> {
    let today = queries::today_day(crt);
    let now = queries::now_unix();
    let decks = queries::get_decks(&db.conn)?;
    let deck_list: Vec<(Deck, u32, u32, u32)> = decks.into_iter().map(|d| {
        let (n, l, r) = queries::get_due_counts(&db.conn, d.id, today, now, crt, review_limit).unwrap_or((0, 0, 0));
        (d, n, l, r)
    }).collect();
    Ok(DeckSelectScreen::new(deck_list))
}
