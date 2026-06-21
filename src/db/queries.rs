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

/// Create a new normal deck and return its id.
///
/// Uses minimal protobuf blobs that Anki accepts for a default normal deck:
/// - `kind`: DeckKind { normal: NormalDeck {} } → [0x0a, 0x00]
/// - `common`: DeckCommon {} → [] (all fields default)
pub fn create_deck(conn: &Connection, name: &str) -> Result<i64> {
    let id = now_ms();
    let kind: &[u8] = &[0x0a, 0x00];
    let common: &[u8] = &[];
    conn.execute(
        "INSERT INTO decks (id, name, mtime_secs, usn, common, kind) \
         VALUES (?1, ?2, ?3, -1, ?4, ?5)",
        params![id, name, now_unix(), common, kind],
    )?;
    Ok(id)
}

pub fn get_due_counts(conn: &Connection, deck_id: i64, today: i64, now: i64) -> Result<(u32, u32, u32)> {
    let new: u32 = conn.query_row(
        "SELECT COUNT(*) FROM cards WHERE did=?1 AND queue=0",
        params![deck_id], |r| r.get(0))?;
    let learning: u32 = conn.query_row(
        "SELECT COUNT(*) FROM cards WHERE did=?1 AND queue=1 AND due<=?2",
        params![deck_id, now], |r| r.get(0))?;
    let review: u32 = conn.query_row(
        "SELECT COUNT(*) FROM cards WHERE did=?1 AND queue=2 AND due<=?2",
        params![deck_id, today], |r| r.get(0))?;
    Ok((new, learning, review))
}

// ── Cards ─────────────────────────────────────────────────────────────────────

fn row_to_card(row: &rusqlite::Row<'_>) -> rusqlite::Result<Card> {
    Ok(Card {
        id: row.get(0)?,
        nid: row.get(1)?,
        did: row.get(2)?,
        ord: row.get(3)?,
        card_type: CardType::try_from(row.get::<_, i32>(4)?).unwrap_or(CardType::New),
        queue: Queue::try_from(row.get::<_, i32>(5)?).unwrap_or(Queue::New),
        due: row.get(6)?,
        ivl: row.get(7)?,
        factor: row.get(8)?,
        reps: row.get(9)?,
        lapses: row.get(10)?,
        data: row.get::<_, Option<String>>(11)?.unwrap_or_default(),
    })
}

pub fn get_due_cards(conn: &Connection, deck_id: i64, today: i64) -> Result<Vec<Card>> {
    let mut stmt = conn.prepare(
        "SELECT id,nid,did,ord,type,queue,due,ivl,factor,reps,lapses,data \
         FROM cards WHERE did=?1 AND queue=2 AND due<=?2 ORDER BY due")?;
    let cards: Vec<Card> = stmt.query_map(params![deck_id, today], row_to_card)?
        .filter_map(|r| r.ok()).collect();
    Ok(cards)
}

pub fn get_learning_cards(conn: &Connection, deck_id: i64, now: i64) -> Result<Vec<Card>> {
    let mut stmt = conn.prepare(
        "SELECT id,nid,did,ord,type,queue,due,ivl,factor,reps,lapses,data \
         FROM cards WHERE did=?1 AND queue=1 AND due<=?2 ORDER BY due")?;
    let cards: Vec<Card> = stmt.query_map(params![deck_id, now], row_to_card)?
        .filter_map(|r| r.ok()).collect();
    Ok(cards)
}

pub fn get_new_cards(conn: &Connection, deck_id: i64, limit: i64) -> Result<Vec<Card>> {
    let mut stmt = conn.prepare(
        "SELECT id,nid,did,ord,type,queue,due,ivl,factor,reps,lapses,data \
         FROM cards WHERE did=?1 AND queue=0 ORDER BY due LIMIT ?2")?;
    let cards: Vec<Card> = stmt.query_map(params![deck_id, limit], row_to_card)?
        .filter_map(|r| r.ok()).collect();
    Ok(cards)
}

// ── Notes & NoteTypes ─────────────────────────────────────────────────────────

pub fn get_note(conn: &Connection, nid: i64) -> Result<Note> {
    Ok(conn.query_row(
        "SELECT id,guid,mid,flds,tags FROM notes WHERE id=?1",
        params![nid],
        |row| Ok(Note {
            id: row.get(0)?,
            guid: row.get(1)?,
            mid: row.get(2)?,
            flds: row.get(3)?,
            tags: row.get(4)?,
        }),
    )?)
}

/// Detect if a protobuf NoteTypeConfig blob encodes kind=Cloze.
/// The protobuf field tag for kind (field 1, varint) is 0x08; cloze = 0x01.
fn is_cloze_config(blob: &[u8]) -> bool {
    blob.first() == Some(&0x08) && blob.get(1) == Some(&0x01)
}

pub fn get_notetype(conn: &Connection, mid: i64) -> Result<NoteType> {
    let (id, name, config_blob): (i64, String, Vec<u8>) = conn.query_row(
        "SELECT id, name, config FROM notetypes WHERE id=?1",
        params![mid],
        |row| Ok((row.get(0)?, row.get(1)?, row.get::<_, Vec<u8>>(2)?)),
    )?;

    let kind = if is_cloze_config(&config_blob) { NoteKind::Cloze } else { NoteKind::Standard };

    let mut fld_stmt = conn.prepare(
        "SELECT ord, name FROM fields WHERE ntid=?1 ORDER BY ord")?;
    let fields: Vec<FieldDef> = fld_stmt.query_map(params![mid], |row| {
        Ok(FieldDef { ord: row.get(0)?, name: row.get(1)? })
    })?
    .filter_map(|r| r.ok())
    .collect();

    let mut tmpl_stmt = conn.prepare(
        "SELECT ord, name FROM templates WHERE ntid=?1 ORDER BY ord")?;
    let templates: Vec<Template> = tmpl_stmt.query_map(params![mid], |row| {
        Ok(Template { ord: row.get(0)?, name: row.get(1)? })
    })?
    .filter_map(|r| r.ok())
    .collect();

    Ok(NoteType { id, name, kind, fields, templates })
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

pub fn write_review(conn: &Connection, result: &ReviewResult, crt: i64) -> Result<()> {
    let s = &result.new_state;
    let today = today_day(crt);
    let due = today + s.due_days;

    let fsrs_json = serde_json::json!({ "s": s.stability, "d": s.difficulty }).to_string();

    conn.execute(
        "UPDATE cards SET type=?1, queue=?2, due=?3, ivl=?4, factor=?5, \
         reps=?6, lapses=?7, mod=?8, usn=-1, data=?9 WHERE id=?10",
        params![
            s.card_type, s.queue, due, s.interval, s.factor,
            s.new_reps, s.new_lapses, now_unix(), fsrs_json, result.card_id,
        ],
    )?;

    conn.execute(
        "INSERT INTO revlog (id, cid, usn, ease, ivl, lastIvl, factor, time, type) \
         VALUES (?1, ?2, -1, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            result.reviewed_at_ms, result.card_id, result.rating,
            s.interval, result.old_ivl, s.factor, result.time_taken_ms, result.old_type,
        ],
    )?;

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

fn extract_cloze_ords(text: &str) -> Vec<u32> {
    let re = regex::Regex::new(r"\{\{c(\d+)::").unwrap();
    let mut ords: Vec<u32> = re.captures_iter(text)
        .filter_map(|c| c[1].parse::<u32>().ok().map(|n| n - 1))
        .collect();
    ords.sort_unstable();
    ords.dedup();
    ords
}
