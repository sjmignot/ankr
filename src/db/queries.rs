use rusqlite::{Connection, params};
use crate::error::Result;
use crate::models::*;

// ── Collection metadata ───────────────────────────────────────────────────────

/// Unix timestamp of collection creation date.
pub fn get_collection_crt(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("SELECT crt FROM col LIMIT 1", [], |r| r.get(0))?)
}

pub fn today_day(crt: i64) -> i64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    (now - crt) / 86400
}

pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

// ── Decks ─────────────────────────────────────────────────────────────────────

pub fn get_decks(conn: &Connection) -> Result<Vec<Deck>> {
    let mut stmt = conn.prepare("SELECT id, name FROM decks WHERE id != 1 ORDER BY name")?;
    let decks = stmt.query_map([], |row| {
        let raw_name: String = row.get(1)?;
        // Anki 2.1.45+ uses \x1f as the hierarchy separator instead of ::
        let name = raw_name.replace('\x1f', "::");
        Ok(Deck { id: row.get(0)?, name })
    })?
    .filter_map(|r| r.ok())
    .collect();
    Ok(decks)
}


/// Resolve a `::` -separated deck path, creating any missing ancestors along
/// the way, and return the leaf deck's id.
///
/// e.g. `"Poetry::Keats::Ode to Autumn"` ensures three decks exist and
/// returns the id of the deepest one.
pub fn get_or_create_deck_path(conn: &Connection, path: &str) -> Result<i64> {
    let mut prefix = String::new();
    let mut last_id = 0i64;
    for (i, part) in path.split("::").enumerate() {
        if i > 0 { prefix.push_str("::"); }
        prefix.push_str(part);
        // DB stores \x1f as hierarchy separator.
        let db_name = prefix.replace("::", "\x1f");
        let existing: Option<i64> = conn
            .query_row("SELECT id FROM decks WHERE name=?1", params![db_name], |r| r.get(0))
            .ok();
        last_id = match existing {
            Some(id) => id,
            None => {
                // Offset by depth to avoid ms-level id collisions.
                let id = now_ms() + i as i64;
                let kind: &[u8] = &[0x0a, 0x00];
                let common: &[u8] = &[];
                conn.execute(
                    "INSERT INTO decks (id, name, mtime_secs, usn, common, kind) \
                     VALUES (?1, ?2, ?3, -1, ?4, ?5)",
                    params![id, db_name, now_unix(), common, kind],
                )?;
                id
            }
        };
    }
    Ok(last_id)
}

/// Reviews already logged today for a deck (used to compute remaining daily allowance).
pub fn get_today_review_count(conn: &Connection, deck_id: i64, crt: i64, today: i64) -> u32 {
    let today_start_ms = (crt + today * 86400) * 1000;
    conn.query_row(
        "SELECT COUNT(*) FROM revlog r \
         JOIN cards c ON c.id = r.cid \
         WHERE c.did=?1 AND r.id>=?2 AND r.type IN (1,2)",
        params![deck_id, today_start_ms],
        |r| r.get(0),
    ).unwrap_or(0)
}

pub fn get_due_counts(conn: &Connection, deck_id: i64, today: i64, now: i64, crt: i64, review_limit: u32) -> Result<(u32, u32, u32)> {
    let new: u32 = conn.query_row(
        "SELECT COUNT(*) FROM cards WHERE did=?1 AND queue=0",
        params![deck_id], |r| r.get(0))?;
    let learning: u32 = conn.query_row(
        "SELECT COUNT(*) FROM cards WHERE did=?1 AND queue=1 AND due<=?2",
        params![deck_id, now], |r| r.get(0))?;
    let total_due: u32 = conn.query_row(
        "SELECT COUNT(*) FROM cards WHERE did=?1 AND queue=2 AND due<=?2",
        params![deck_id, today], |r| r.get(0))?;
    let done_today = get_today_review_count(conn, deck_id, crt, today);
    let review = total_due.min(review_limit.saturating_sub(done_today));
    Ok((new, learning, review))
}

// ── Cards ─────────────────────────────────────────────────────────────────────

fn row_to_card(row: &rusqlite::Row<'_>) -> rusqlite::Result<Card> {
    Ok(Card {
        id: row.get(0)?,
        nid: row.get(1)?,
        ord: row.get(2)?,
        card_type: CardType::try_from(row.get::<_, i32>(3)?).unwrap_or(CardType::New),
        ivl: row.get(4)?,
        factor: row.get(5)?,
        reps: row.get(6)?,
        lapses: row.get(7)?,
        data: row.get::<_, Option<String>>(8)?.unwrap_or_default(),
    })
}

pub fn get_due_cards(conn: &Connection, deck_id: i64, today: i64, crt: i64, review_limit: u32) -> Result<Vec<Card>> {
    let done_today = get_today_review_count(conn, deck_id, crt, today);
    let remaining = review_limit.saturating_sub(done_today) as i64;
    let mut stmt = conn.prepare(
        "SELECT id,nid,ord,type,ivl,factor,reps,lapses,data \
         FROM cards WHERE did=?1 AND queue=2 AND due<=?2 ORDER BY due LIMIT ?3")?;
    let cards: Vec<Card> = stmt.query_map(params![deck_id, today, remaining], row_to_card)?
        .filter_map(|r| r.ok()).collect();
    Ok(cards)
}

pub fn get_learning_cards(conn: &Connection, deck_id: i64, now: i64) -> Result<Vec<Card>> {
    let mut stmt = conn.prepare(
        "SELECT id,nid,ord,type,ivl,factor,reps,lapses,data \
         FROM cards WHERE did=?1 AND queue=1 AND due<=?2 ORDER BY due")?;
    let cards: Vec<Card> = stmt.query_map(params![deck_id, now], row_to_card)?
        .filter_map(|r| r.ok()).collect();
    Ok(cards)
}

pub fn get_new_cards(conn: &Connection, deck_id: i64, limit: i64) -> Result<Vec<Card>> {
    let mut stmt = conn.prepare(
        "SELECT id,nid,ord,type,ivl,factor,reps,lapses,data \
         FROM cards WHERE did=?1 AND queue=0 ORDER BY due LIMIT ?2")?;
    let cards: Vec<Card> = stmt.query_map(params![deck_id, limit], row_to_card)?
        .filter_map(|r| r.ok()).collect();
    Ok(cards)
}

// ── Notes & NoteTypes ─────────────────────────────────────────────────────────

pub fn get_note(conn: &Connection, nid: i64) -> Result<Note> {
    Ok(conn.query_row(
        "SELECT id,mid,flds,tags FROM notes WHERE id=?1",
        params![nid],
        |row| Ok(Note {
            id: row.get(0)?,
            mid: row.get(1)?,
            flds: row.get(2)?,
            tags: row.get(3)?,
        }),
    )?)
}

/// Detect if a protobuf NoteTypeConfig blob encodes kind=Cloze.
/// The protobuf field tag for kind (field 1, varint) is 0x08; cloze = 0x01.
fn is_cloze_config(blob: &[u8]) -> bool {
    blob.first() == Some(&0x08) && blob.get(1) == Some(&0x01)
}

pub fn get_notetype(conn: &Connection, mid: i64) -> Result<NoteType> {
    let (name, config_blob): (String, Vec<u8>) = conn.query_row(
        "SELECT name, config FROM notetypes WHERE id=?1",
        params![mid],
        |row| Ok((row.get(0)?, row.get::<_, Vec<u8>>(1)?)),
    )?;

    let kind = if is_cloze_config(&config_blob) { NoteKind::Cloze } else { NoteKind::Standard };

    let mut fld_stmt = conn.prepare(
        "SELECT name FROM fields WHERE ntid=?1 ORDER BY ord")?;
    let fields: Vec<FieldDef> = fld_stmt.query_map(params![mid], |row| {
        Ok(FieldDef { name: row.get(0)? })
    })?
    .filter_map(|r| r.ok())
    .collect();

    let mut tmpl_stmt = conn.prepare(
        "SELECT ord, config FROM templates WHERE ntid=?1 ORDER BY ord")?;
    let templates: Vec<Template> = tmpl_stmt.query_map(params![mid], |row| {
        let config: Vec<u8> = row.get(1)?;
        let (qfmt, afmt) = decode_template_config(&config);
        Ok(Template { ord: row.get(0)?, qfmt, afmt })
    })?
    .filter_map(|r| r.ok())
    .collect();

    Ok(NoteType { name, kind, fields, templates })
}

pub fn get_resolved_note(conn: &Connection, card: &Card) -> Result<ResolvedNote> {
    let note = get_note(conn, card.nid)?;
    let notetype = get_notetype(conn, note.mid)?;
    let raw_fields: Vec<&str> = note.flds.split('\x1f').collect();
    let fields = notetype.fields.iter().enumerate().map(|(i, f)| {
        let val = raw_fields.get(i).copied().unwrap_or("").to_string();
        (f.name.clone(), val)
    }).collect();
    let tags = note.tags.split_whitespace().map(|s| s.to_string()).collect();
    Ok(ResolvedNote { id: note.id, notetype, fields, tags })
}

// ── Notetypes for creation ────────────────────────────────────────────────────

pub fn get_all_notetypes(conn: &Connection) -> Result<Vec<(i64, String)>> {
    let mut stmt = conn.prepare("SELECT id, name FROM notetypes ORDER BY name")?;
    let result: Vec<(i64, String)> = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .filter_map(|r| r.ok()).collect();
    Ok(result)
}

pub fn get_cloze_notetype_id(conn: &Connection) -> Result<Option<i64>> {
    let mut stmt = conn.prepare("SELECT id, config FROM notetypes")?;
    let id = stmt.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
    })?
    .filter_map(|r| r.ok())
    .find(|(_, blob)| is_cloze_config(blob))
    .map(|(id, _)| id);
    Ok(id)
}

// ── Write review ──────────────────────────────────────────────────────────────

/// Stamp col.mod so Anki detects external changes and triggers auto-sync on next open.
fn touch_col(conn: &Connection) -> Result<()> {
    conn.execute("UPDATE col SET mod=?1", params![now_unix()])?;
    Ok(())
}

pub fn write_review(conn: &Connection, result: &ReviewResult, crt: i64) -> Result<()> {
    let s = &result.new_state;
    let today = today_day(crt);
    let due = today + s.due_days;
    let now = now_unix();

    // Merge into existing data: preserve Anki's `pos` field and add `lrt`
    // (last-review-time, Unix seconds) which Anki requires. Also carry our
    // FSRS state under `s`/`d` for scheduling on subsequent reviews.
    let existing: String = conn
        .query_row("SELECT data FROM cards WHERE id=?1", params![result.card_id], |r| r.get(0))
        .unwrap_or_default();
    let mut data: serde_json::Value =
        serde_json::from_str(&existing).unwrap_or(serde_json::json!({}));
    data["lrt"] = serde_json::json!(now);
    // Guard against NaN (produced when stability=0 fed into FSRS review formula).
    if (s.stability as f64).is_finite() {
        data["s"] = serde_json::json!(s.stability);
    }
    if (s.difficulty as f64).is_finite() {
        data["d"] = serde_json::json!(s.difficulty);
    }
    let data_str = data.to_string();

    conn.execute(
        "UPDATE cards SET type=?1, queue=?2, due=?3, ivl=?4, factor=?5, \
         reps=?6, lapses=?7, mod=?8, usn=-1, data=?9 WHERE id=?10",
        params![
            s.card_type, s.queue, due, s.interval, s.factor,
            s.new_reps, s.new_lapses, now, data_str, result.card_id,
        ],
    )?;

    // Anki revlog type: 0=Learning, 1=Review, 2=Relearning, 3=Filtered.
    // CardType enum values don't align (Review=2 vs revlog Review=1), so map explicitly.
    let revlog_type: i32 = match result.old_type {
        0 | 1 => 0, // New/Learning → Learning
        2 => 1,     // Review → Review
        3 => 2,     // Relearning → Relearning
        _ => 1,
    };
    conn.execute(
        "INSERT INTO revlog (id, cid, usn, ease, ivl, lastIvl, factor, time, type) \
         VALUES (?1, ?2, -1, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            result.reviewed_at_ms, result.card_id, result.rating,
            s.interval, result.old_ivl, s.factor, result.time_taken_ms, revlog_type,
        ],
    )?;

    touch_col(conn)?;
    Ok(())
}

// ── Card creation ─────────────────────────────────────────────────────────────

pub fn insert_note(conn: &Connection, note: &NewCard) -> Result<i64> {
    let now = now_unix();
    let id = now_ms();
    let flds = if note.back.is_empty() {
        note.text.clone()
    } else {
        format!("{}\x1f{}", note.text, note.back)
    };
    let tags = note.tags.join(" ");
    // Simple sort-field checksum
    let sfld = note.text.chars().take(20).collect::<String>();
    let csum: i64 = sfld.bytes().fold(0i64, |a, b| a.wrapping_add(b as i64));

    conn.execute(
        "INSERT INTO notes (id, guid, mid, mod, usn, tags, flds, sfld, csum, flags, data) \
         VALUES (?1, ?2, ?3, ?4, -1, ?5, ?6, ?7, ?8, 0, '')",
        params![id, format!("{id:x}"), note.notetype_id, now, tags, flds, sfld, csum],
    )?;

    let nid = conn.last_insert_rowid();
    let notetype = get_notetype(conn, note.notetype_id)?;

    match notetype.kind {
        NoteKind::Cloze => {
            for ord in extract_cloze_ords(&note.text) {
                insert_card(conn, nid, note.deck_id, ord as i32, now)?;
            }
        }
        NoteKind::Standard => {
            for tmpl in &notetype.templates {
                insert_card(conn, nid, note.deck_id, tmpl.ord, now)?;
            }
        }
    }

    touch_col(conn)?;
    Ok(nid)
}

fn insert_card(conn: &Connection, nid: i64, did: i64, ord: i32, now: i64) -> Result<()> {
    let id = now_ms() + ord as i64;
    conn.execute(
        "INSERT INTO cards (id, nid, did, ord, mod, usn, type, queue, due, ivl, factor, \
         reps, lapses, left, odue, odid, flags, data) \
         VALUES (?1, ?2, ?3, ?4, ?5, -1, 0, 0, ?1, 0, 2500, 0, 0, 0, 0, 0, 0, '')",
        params![id, nid, did, ord, now],
    )?;
    Ok(())
}

/// Minimal protobuf varint decoder. Returns (value, bytes_consumed).
fn read_varint(buf: &[u8]) -> (u64, usize) {
    let mut result: u64 = 0;
    let mut shift = 0u32;
    for (i, &byte) in buf.iter().enumerate() {
        result |= ((byte & 0x7F) as u64) << shift;
        shift += 7;
        if byte & 0x80 == 0 {
            return (result, i + 1);
        }
    }
    (result, buf.len())
}

/// Decode Anki's CardTemplateConfig protobuf blob into (qfmt, afmt).
/// Field 1 = qfmt (question template), field 2 = afmt (answer template).
fn decode_template_config(blob: &[u8]) -> (String, String) {
    let mut qfmt = String::new();
    let mut afmt = String::new();
    let mut i = 0;
    while i < blob.len() {
        let (tag, c) = read_varint(&blob[i..]);
        i += c;
        let wire = tag & 0x7;
        let field = tag >> 3;
        match wire {
            0 => { let (_, c) = read_varint(&blob[i..]); i += c; }
            1 => { i += 8; }
            2 => {
                let (len, c) = read_varint(&blob[i..]);
                i += c;
                let end = (i + len as usize).min(blob.len());
                let s = String::from_utf8_lossy(&blob[i..end]).into_owned();
                match field { 1 => qfmt = s, 2 => afmt = s, _ => {} }
                i = end;
            }
            5 => { i += 4; }
            _ => break,
        }
    }
    (qfmt, afmt)
}

fn extract_cloze_ords(text: &str) -> Vec<u32> {
    let re = regex::Regex::new(r"\{\{c(\d+)::").unwrap();
    let mut ords: Vec<u32> = re.captures_iter(text)
        .filter_map(|c| c[1].parse::<u32>().ok().map(|n| n - 1))
        .collect();
    ords.sort_unstable();
    ords.dedup();
    ords
}
