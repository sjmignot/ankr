pub mod import;
pub mod lock;
pub mod queries;

use std::path::{Path, PathBuf};
use rusqlite::{Connection, OpenFlags};
use anyhow::Context;
use crate::error::{AnkrError, Result};

pub struct DbConn {
    pub conn: Connection,
}

impl DbConn {
    pub fn open(db_path: &Path, readonly: bool) -> Result<Self> {
        let flags = if readonly {
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX
        } else {
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX
        };

        let conn = Connection::open_with_flags(db_path, flags)
            .with_context(|| format!("opening {}", db_path.display()))
            .map_err(AnkrError::Other)?;

        // Anki uses a custom unicase collation for case-insensitive text columns.
        // Register it as a simple Unicode lowercase comparison.
        conn.create_collation("unicase", |a, b| {
            a.to_lowercase().cmp(&b.to_lowercase())
        })?;

        if !readonly {
            conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        }
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;

        Ok(Self { conn })
    }
}

pub fn default_collection_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("~/.local/share"))
        .join("Anki2")
        .join("User 1")
        .join("collection.anki2")
}

pub fn media_dir(db_path: &Path) -> PathBuf {
    db_path.parent().unwrap_or(db_path).join("collection.media")
}
