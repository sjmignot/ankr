use thiserror::Error;

#[derive(Debug, Error)]
pub enum AnkrError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("Anki is currently open — close it before writing reviews")]
    AnkiLocked,

    #[error("collection not found at {0}")]
    CollectionNotFound(String),

    #[error("AI error: {0}")]
    Ai(String),

    #[error("parse error: {0}")]
    Parse(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, AnkrError>;
