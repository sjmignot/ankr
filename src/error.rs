use thiserror::Error;

#[derive(Debug, Error)]
pub enum AnkrError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("AI error: {0}")]
    Ai(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, AnkrError>;
