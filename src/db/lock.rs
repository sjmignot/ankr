use std::path::Path;
use rusqlite::{Connection, OpenFlags};

/// Returns true if Anki (or another process) holds an exclusive lock on the collection.
pub fn is_collection_locked(db_path: &Path) -> bool {
    let Ok(conn) = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return true;
    };
    // A successful BEGIN EXCLUSIVE means we got the write lock; roll back immediately.
    conn.execute_batch("BEGIN EXCLUSIVE; ROLLBACK;").is_err()
}
