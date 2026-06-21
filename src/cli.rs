use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "ankr", about = "Terminal Anki client", version)]
pub struct Cli {
    /// Deck name (substring match)
    #[arg(short, long)]
    pub deck: Option<String>,

    /// Path to collection.anki2
    #[arg(long)]
    pub db: Option<PathBuf>,

    /// Maximum new cards per session
    #[arg(short = 'n', long, default_value = "20")]
    pub new_limit: u32,

    /// Never write to the database (read-only preview)
    #[arg(long)]
    pub readonly: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Print due card counts per deck and exit
    Stats,

    /// Sync the collection with AnkiWeb (no Anki desktop needed)
    Sync {
        /// AnkiWeb username / email (or set ANKIWEB_USER env var)
        #[arg(short, long, env = "ANKIWEB_USER")]
        username: Option<String>,

        /// AnkiWeb password (or set ANKIWEB_PASS env var)
        #[arg(short, long, env = "ANKIWEB_PASS")]
        password: Option<String>,
    },

    /// Create Anki cards from a poem using the LPCG method
    Poem {
        /// Poem text file (reads from stdin if omitted)
        file: Option<PathBuf>,

        /// Deck name (substring match, case-insensitive)
        #[arg(short, long, default_value = "Poetry")]
        deck: String,

        /// Tags to apply (space-separated)
        #[arg(short, long, default_value = "")]
        tags: String,

        /// Use stanza mode instead of line mode
        #[arg(long)]
        stanza: bool,

        /// Print cards without writing to the database
        #[arg(long)]
        dry_run: bool,
    },
}
