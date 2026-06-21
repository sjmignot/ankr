pub mod events;
pub mod screens;

use std::io;
use std::path::PathBuf;
use std::time::Duration;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, enable_raw_mode, disable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use ratatui_image::picker::Picker;
use tokio::sync::mpsc;

use crate::db::{DbConn, queries};
use crate::error::Result;
use crate::models::*;
use crate::scheduler::FsrsScheduler;
use crate::review::ReviewQueue;
use crate::ai::ClaudeClient;
use screens::{AiCreateScreen, CreateScreen, DeckSelectScreen, DoneScreen, ReviewScreen};
use screens::ai_create::{AiAction, AiState};
use screens::deck_select::DeckAction;
use screens::review::ReviewAction;
use screens::create::CreateAction;

pub struct AppConfig {
    pub db_path: PathBuf,
    pub media_dir: PathBuf,
    pub new_limit: u32,
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
        notetype_id: i64,
    },
    Done(DoneScreen),
    Create(CreateScreen),
    AiCreate(AiCreateScreen),
}

pub async fn run(db: DbConn, config: AppConfig) -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let picker = Picker::from_query_stdio().ok();

    let result = run_app(&mut terminal, db, config, picker).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;

    result
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    db: DbConn,
    config: AppConfig,
    picker: Option<Picker>,
) -> anyhow::Result<()> {
    let crt = queries::get_collection_crt(&db.conn)?;

    let (ai_tx, mut ai_rx) = mpsc::channel::<std::result::Result<Vec<NewCard>, String>>(4);

    let mut screen = Screen::DeckSelect(build_deck_select(&db, crt)?);

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
            }
        })?;

        // Poll event (100ms so spinner animates)
        let maybe_key = tokio::task::spawn_blocking(|| {
            if crossterm::event::poll(Duration::from_millis(100)).unwrap_or(false) {
                crossterm::event::read().ok().and_then(|e| {
                    if let crossterm::event::Event::Key(k) = e { Some(k) } else { None }
                })
            } else {
                None
            }
        }).await?;

        let Some(key) = maybe_key else { continue };

        match &mut screen {
            // ── Deck Select ──────────────────────────────────────────────────
            Screen::DeckSelect(s) => match s.handle_key(&key) {
                DeckAction::Quit => break,
                DeckAction::None => {}
                DeckAction::Select => {
                    if let Some(deck) = s.selected_deck() {
                        let deck_id = deck.id;
                        let today = queries::today_day(crt);
                        let now = queries::now_unix();

                        let learning = queries::get_learning_cards(&db.conn, deck_id, now)?;
                        let due = queries::get_due_cards(&db.conn, deck_id, today)?;
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
                                screen: ReviewScreen::new(card, note, picker.clone(), config.media_dir.clone()),
                                queue,
                                scheduler: FsrsScheduler::new(0.9),
                                crt,
                                deck_id,
                                notetype_id,
                            };
                        }
                    }
                }
            },

            // ── Review ───────────────────────────────────────────────────────
            Screen::Review { screen: rev, queue, scheduler, crt: review_crt, deck_id, notetype_id } => {
                match key.code {
                    KeyCode::Char('c') => {
                        screen = Screen::Create(CreateScreen::new(*deck_id, *notetype_id));
                        continue;
                    }
                    KeyCode::Char('a') => {
                        screen = Screen::AiCreate(AiCreateScreen::new(*deck_id, *notetype_id));
                        continue;
                    }
                    _ => {}
                }

                match rev.handle_key(&key) {
                    ReviewAction::Quit => break,
                    ReviewAction::None => {}
                    ReviewAction::Back => {
                        screen = Screen::DeckSelect(build_deck_select(&db, crt)?);
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
                            *rev = ReviewScreen::new(next_card, note, picker.clone(), config.media_dir.clone());
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
                    screen = Screen::DeckSelect(build_deck_select(&db, crt)?);
                }
            }

            // ── Manual Create ─────────────────────────────────────────────────
            Screen::Create(s) => match s.handle_key(key) {
                CreateAction::None => {}
                CreateAction::Cancel => {
                    screen = Screen::DeckSelect(build_deck_select(&db, crt)?);
                }
                CreateAction::Save(card) => {
                    if !config.readonly {
                        queries::insert_note(&db.conn, &card)?;
                    }
                    screen = Screen::DeckSelect(build_deck_select(&db, crt)?);
                }
            },

            // ── AI Create ─────────────────────────────────────────────────────
            Screen::AiCreate(s) => match s.handle_key(key) {
                AiAction::None => {}
                AiAction::Cancel => {
                    screen = Screen::DeckSelect(build_deck_select(&db, crt)?);
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
                        screen = Screen::DeckSelect(build_deck_select(&db, crt)?);
                    }
                }
                AiAction::SkipCard => {
                    if matches!(s.state, AiState::Done) {
                        screen = Screen::DeckSelect(build_deck_select(&db, crt)?);
                    }
                }
            },
        }
    }

    Ok(())
}

fn build_deck_select(db: &DbConn, crt: i64) -> anyhow::Result<DeckSelectScreen> {
    let today = queries::today_day(crt);
    let now = queries::now_unix();
    let decks = queries::get_decks(&db.conn)?;
    let deck_list: Vec<(Deck, u32, u32, u32)> = decks.into_iter().map(|d| {
        let (n, l, r) = queries::get_due_counts(&db.conn, d.id, today, now).unwrap_or((0, 0, 0));
        (d, n, l, r)
    }).collect();
    Ok(DeckSelectScreen::new(deck_list))
}
