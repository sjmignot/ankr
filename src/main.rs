mod ai;
mod cli;
mod config;
mod db;
mod error;
mod models;
mod render;
mod review;
mod scheduler;
mod sync;
mod tui;

use clap::Parser;
use cli::{Cli, Commands};
use db::{DbConn, default_collection_path, media_dir, queries};
use render::poem::{GranularityMode, poem_to_lpcg};
use models::NewCard;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let db_path = cli.db.unwrap_or_else(default_collection_path);
    if !db_path.exists() {
        anyhow::bail!("Collection not found: {}", db_path.display());
    }

    let locked = db::lock::is_collection_locked(&db_path);
    if locked && !cli.readonly {
        eprintln!(
            "Warning: Anki appears to be open. Reviews will be written but may conflict.\n\
             Use --readonly to suppress this warning."
        );
    }

    let media = media_dir(&db_path);
    let db = DbConn::open(&db_path, cli.readonly)?;

    match cli.command {
        Some(Commands::Stats) => {
            let crt = queries::get_collection_crt(&db.conn)?;
            let today = queries::today_day(crt);
            let now = queries::now_unix();
            let decks = queries::get_decks(&db.conn)?;
            if decks.is_empty() {
                println!("No decks found.");
                return Ok(());
            }
            println!("{:<40} {:>5} {:>5} {:>5}", "Deck", "New", "Lrn", "Rev");
            println!("{}", "-".repeat(57));
            for deck in &decks {
                let (new, learning, review) = queries::get_due_counts(&db.conn, deck.id, today, now, crt, cli.review_limit)
                    .unwrap_or((0, 0, 0));
                println!("{:<40} {:>5} {:>5} {:>5}", deck.name, new, learning, review);
            }
        }
        Some(Commands::Sync { username, password }) => {
            let cfg = config::load();

            // Resolve username: flag > env > config file > prompt.
            let username = match username {
                Some(u) => u,
                None => match cfg.sync.username.clone() {
                    Some(u) => u,
                    None => {
                        eprint!("AnkiWeb email: ");
                        let mut s = String::new();
                        std::io::stdin().read_line(&mut s)?;
                        s.trim().to_string()
                    }
                },
            };

            // Resolve password: flag > env > config file > prompt.
            let password = match password {
                Some(p) => p,
                None => match cfg.sync.password.clone() {
                    Some(p) => p,
                    None => {
                        eprint!("AnkiWeb password: ");
                        match rpassword::read_password() {
                            Ok(p) => p,
                            Err(_) => {
                                let mut s = String::new();
                                std::io::stdin().read_line(&mut s)?;
                                s.trim().to_string()
                            }
                        }
                    }
                },
            };

            // Offer to save credentials if they weren't already in the config.
            if cfg.sync.username.is_none() || cfg.sync.password.is_none() {
                eprint!("Save credentials to {}? [y/N] ", config::config_path().display());
                let mut ans = String::new();
                std::io::stdin().read_line(&mut ans)?;
                if ans.trim().eq_ignore_ascii_case("y") {
                    let new_cfg = config::Config {
                        sync: config::SyncConfig {
                            username: Some(username.clone()),
                            password: Some(password.clone()),
                        },
                    };
                    config::save(&new_cfg)?;
                    eprintln!("Credentials saved.");
                }
            }

            eprintln!("Logging in to AnkiWeb…");
            let client = sync::SyncClient::login(&username, &password).await?;
            eprintln!("Syncing…");
            let summary = client.sync(&db.conn).await?;
            println!(
                "Sync complete — pushed {} card(s), {} revlog(s), {} note(s); pulled {} card(s) from server.",
                summary.pushed_cards, summary.pushed_revlog, summary.pushed_notes, summary.pulled_cards,
            );
            if !summary.server_msg.is_empty() {
                println!("Server message: {}", summary.server_msg);
            }
        }

        Some(Commands::Import { file }) => {
            if cli.readonly {
                anyhow::bail!("Cannot import in --readonly mode.");
            }
            if !file.exists() {
                anyhow::bail!("File not found: {}", file.display());
            }
            eprintln!("Importing {}…", file.display());
            let n = db::import::import_apkg(&db.conn, &file)?;
            println!("Imported {n} new note(s). Run `ankr sync` to push to AnkiWeb.");
        }

        Some(Commands::Ai { deck }) => {
            let decks = queries::get_decks(&db.conn)?;
            let matched = decks.iter().find(|d| {
                d.name.to_lowercase().contains(&deck.to_lowercase())
            });
            let deck_entry = match matched {
                Some(d) => d.clone(),
                None => {
                    let id = queries::get_or_create_deck_path(&db.conn, &deck)?;
                    eprintln!("Created deck \"{}\".", deck);
                    models::Deck { id, name: deck.clone() }
                }
            };
            let notetype_id = queries::get_cloze_notetype_id(&db.conn)?
                .ok_or_else(|| anyhow::anyhow!("No cloze notetype found in collection."))?;
            tui::run_ai(db, deck_entry.id, notetype_id, cli.readonly).await?;
        }

        Some(Commands::Poem { file, deck, tags, stanza, dry_run }) => {
            use std::io::IsTerminal;

            // Find deck by substring match (needed for both TUI and CLI paths)
            let decks = queries::get_decks(&db.conn)?;
            let matched = decks.iter().find(|d| {
                d.name.to_lowercase().contains(&deck.to_lowercase())
            });
            let deck_entry = match matched {
                Some(d) => d.clone(),
                None => {
                    let id = queries::get_or_create_deck_path(&db.conn, &deck)?;
                    eprintln!("Created deck \"{}\".", deck);
                    models::Deck { id, name: deck.clone() }
                }
            };
            let notetype_id = queries::get_cloze_notetype_id(&db.conn)?
                .ok_or_else(|| anyhow::anyhow!("No cloze notetype found in collection."))?;

            // No file and stdin is a terminal → open TUI poem screen
            if file.is_none() && std::io::stdin().is_terminal() {
                tui::run_poem(db, deck_entry.name.clone(), deck_entry.id, notetype_id, cli.readonly).await?;
                return Ok(());
            }

            let text = match file {
                Some(path) => std::fs::read_to_string(&path)
                    .map_err(|e| anyhow::anyhow!("Could not read {}: {e}", path.display()))?,
                None => {
                    let mut buf = String::new();
                    std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)?;
                    buf
                }
            };

            let text = text.trim().to_string();
            if text.is_empty() {
                anyhow::bail!("No poem text provided.");
            }

            let mode = if stanza { GranularityMode::Stanza } else { GranularityMode::Line };
            let tag_list: Vec<String> = tags.split_whitespace().map(|s| s.to_string()).collect();

            let cards: Vec<NewCard> = poem_to_lpcg(&text, mode)
                .into_iter()
                .map(|card_text| NewCard {
                    text: card_text,
                    back: String::new(),
                    tags: tag_list.clone(),
                    deck_id: deck_entry.id,
                    notetype_id,
                })
                .collect();

            if dry_run {
                for (i, card) in cards.iter().enumerate() {
                    println!("--- Card {} ---\n{}\n", i + 1, card.text);
                }
                println!("({} cards, dry run — nothing written)", cards.len());
            } else {
                for card in &cards {
                    queries::insert_note(&db.conn, card)?;
                }
                println!("Created {} cards in \"{}\".", cards.len(), deck_entry.name);
            }
        }

        None => {
            let config = tui::AppConfig {
                media_dir: media,
                new_limit: cli.new_limit,
                review_limit: cli.review_limit,
                readonly: cli.readonly,
            };
            tui::run(db, config).await?;
        }
    }

    Ok(())
}
