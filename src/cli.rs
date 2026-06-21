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
}
