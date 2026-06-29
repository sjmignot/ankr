use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::Path;
use rusqlite::{Connection, OpenFlags, params};
use anyhow::{Context, Result};
use super::queries::{now_ms, now_unix, get_or_create_deck_path};

pub fn import_apkg(conn: &Connection, path: &Path) -> Result<usize> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file)?;

    // .apkg may contain collection.anki21 (newer) or collection.anki2 (older)
    let col_name = if archive.by_name("collection.anki21").is_ok() {
        "collection.anki21"
    } else {
        "collection.anki2"
    };

    let tmp_path = std::env::temp_dir().join("ankr_import_src.anki2");
    let mut entry = archive.by_name(col_name)?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes)?;
    drop(entry);
    std::fs::write(&tmp_path, &bytes)?;

    let src = Connection::open_with_flags(
        &tmp_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;

    let nt_map = import_notetypes(conn, &src)?;
    let deck_map = import_decks(conn, &src)?;
    let count = import_notes(conn, &src, &nt_map, &deck_map)?;

    let _ = std::fs::remove_file(&tmp_path);
    Ok(count)
}

fn import_notetypes(conn: &Connection, src: &Connection) -> Result<HashMap<i64, i64>> {
    let mut dst_by_name: HashMap<String, i64> = HashMap::new();
    let mut stmt = conn.prepare("SELECT id, name FROM notetypes")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_,i64>(0)?, r.get::<_,String>(1)?)))?
        .flatten()
    {
        dst_by_name.insert(row.1, row.0);
    }
    drop(stmt);

    let mut stmt = src.prepare("SELECT id, name, config FROM notetypes")?;
    let src_notetypes: Vec<(i64, String, Vec<u8>)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_,Vec<u8>>(2)?)))?
        .flatten().collect();
    drop(stmt);

    let mut nt_map: HashMap<i64, i64> = HashMap::new();

    for (i, (src_id, name, config)) in src_notetypes.into_iter().enumerate() {
        let dst_id = if let Some(&existing) = dst_by_name.get(&name) {
            existing
        } else {
            let new_id = now_ms() + i as i64;
            conn.execute(
                "INSERT OR IGNORE INTO notetypes (id, name, mtime_secs, usn, config) \
                 VALUES (?1, ?2, ?3, -1, ?4)",
                params![new_id, name, now_unix(), config],
            )?;

            let mut fstmt = src.prepare(
                "SELECT ord, name, config FROM fields WHERE ntid=?1 ORDER BY ord")?;
            let src_fields: Vec<(i32, String, Vec<u8>)> = fstmt
                .query_map(params![src_id], |r| {
                    Ok((r.get::<_,i32>(0)?, r.get::<_,String>(1)?, r.get::<_,Vec<u8>>(2)?))
                })?.flatten().collect();
            drop(fstmt);
            for (ord, fname, fcfg) in src_fields {
                conn.execute(
                    "INSERT OR IGNORE INTO fields (ntid, ord, name, config) VALUES (?1,?2,?3,?4)",
                    params![new_id, ord, fname, fcfg],
                )?;
            }

            let mut tstmt = src.prepare(
                "SELECT ord, name, config FROM templates WHERE ntid=?1 ORDER BY ord")?;
            let src_tmpls: Vec<(i32, String, Vec<u8>)> = tstmt
                .query_map(params![src_id], |r| {
                    Ok((r.get::<_,i32>(0)?, r.get::<_,String>(1)?, r.get::<_,Vec<u8>>(2)?))
                })?.flatten().collect();
            drop(tstmt);
            for (ord, tname, tcfg) in src_tmpls {
                conn.execute(
                    "INSERT OR IGNORE INTO templates (ntid, ord, name, config) VALUES (?1,?2,?3,?4)",
                    params![new_id, ord, tname, tcfg],
                )?;
            }

            dst_by_name.insert(name, new_id);
            new_id
        };
        nt_map.insert(src_id, dst_id);
    }

    Ok(nt_map)
}

fn import_decks(conn: &Connection, src: &Connection) -> Result<HashMap<i64, i64>> {
    let mut stmt = src.prepare("SELECT id, name FROM decks WHERE id != 1 ORDER BY name")?;
    let src_decks: Vec<(i64, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get::<_,String>(1)?)))?
        .flatten()
        .map(|(id, raw)| (id, raw.replace('\x1f', "::")))
        .collect();
    drop(stmt);

    let mut deck_map: HashMap<i64, i64> = HashMap::new();
    for (src_id, name) in src_decks {
        let dst_id = get_or_create_deck_path(conn, &name)?;
        deck_map.insert(src_id, dst_id);
    }
    Ok(deck_map)
}

fn import_notes(
    conn: &Connection,
    src: &Connection,
    nt_map: &HashMap<i64, i64>,
    deck_map: &HashMap<i64, i64>,
) -> Result<usize> {
    let mut stmt = conn.prepare("SELECT guid FROM notes")?;
    let mut seen_guids: HashSet<String> = stmt
        .query_map([], |r| r.get(0))?.flatten().collect();
    drop(stmt);

    let mut stmt = src.prepare(
        "SELECT id, guid, mid, mod, tags, flds, sfld, csum, flags FROM notes"
    )?;
    let src_notes: Vec<(i64, String, i64, i64, String, String, String, i64, i64)> = stmt
        .query_map([], |r| Ok((
            r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?,
            r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?,
        )))?.flatten().collect();
    drop(stmt);

    conn.execute_batch("BEGIN IMMEDIATE")?;
    let result: Result<usize> = (|| -> Result<usize> {
        let mut count = 0usize;

        for (i, (src_nid, guid, src_mid, mod_time, tags, flds, sfld, csum, flags)) in
            src_notes.into_iter().enumerate()
        {
            if seen_guids.contains(&guid) { continue; }
            let Some(&dst_mid) = nt_map.get(&src_mid) else { continue };

            let nid = if conn.query_row(
                "SELECT 1 FROM notes WHERE id=?1", params![src_nid], |_| Ok(())
            ).is_ok() {
                now_ms() + i as i64
            } else {
                src_nid
            };

            conn.execute(
                "INSERT INTO notes \
                 (id,guid,mid,mod,usn,tags,flds,sfld,csum,flags,data) \
                 VALUES (?1,?2,?3,?4,-1,?5,?6,?7,?8,?9,'')",
                params![nid, &guid, dst_mid, mod_time, tags, flds, sfld, csum, flags],
            )?;
            seen_guids.insert(guid);

            let mut cstmt = src.prepare(
                "SELECT id, did, ord, flags FROM cards WHERE nid=?1")?;
            let src_cards: Vec<(i64, i64, i32, i32)> = cstmt
                .query_map(params![src_nid], |r| Ok((
                    r.get(0)?, r.get(1)?, r.get::<_,i32>(2)?, r.get::<_,i32>(3)?,
                )))?.flatten().collect();
            drop(cstmt);

            for (j, (src_cid, src_did, ord, cflags)) in src_cards.into_iter().enumerate() {
                let dst_did = deck_map.get(&src_did).copied().unwrap_or(src_did);
                let cid = if conn.query_row(
                    "SELECT 1 FROM cards WHERE id=?1", params![src_cid], |_| Ok(())
                ).is_ok() {
                    now_ms() + i as i64 * 100 + j as i64
                } else {
                    src_cid
                };
                conn.execute(
                    "INSERT INTO cards \
                     (id,nid,did,ord,mod,usn,type,queue,due,ivl,factor,reps,lapses,\
                      left,odue,odid,flags,data) \
                     VALUES (?1,?2,?3,?4,?5,-1,0,0,?4,0,0,0,0,0,0,0,?6,'')",
                    params![cid, nid, dst_did, ord, now_unix(), cflags],
                )?;
            }

            count += 1;
        }
        Ok(count)
    })();

    match result {
        Ok(n) => {
            conn.execute("UPDATE col SET mod=?1 WHERE 1=1", params![now_unix()])?;
            conn.execute_batch("COMMIT")?;
            Ok(n)
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}
