mod ai;
mod cli;
mod db;
mod error;
mod models;
mod render;
mod review;
mod scheduler;
mod tui;

use clap::Parser;
use cli::{Cli, Commands};
use db::{DbConn, default_collection_path, media_dir, queries};

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
                let (new, learning, review) = queries::get_due_counts(&db.conn, deck.id, today, now)
                    .unwrap_or((0, 0, 0));
                println!("{:<40} {:>5} {:>5} {:>5}", deck.name, new, learning, review);
            }
        }
        None => {
            let config = tui::AppConfig {
                db_path: db_path.clone(),
                media_dir: media,
                new_limit: cli.new_limit,
                readonly: cli.readonly,
            };
            tui::run(db, config).await?;
        }
    }

    Ok(())
}
