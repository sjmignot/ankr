use anyhow::{bail, Context, Result};
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

const BASE: &str = "https://sync.ankiweb.net/sync/";
const SYNC_VER: u64 = 8;
const CLIENT_VER: &str = "ankr,0.1.0,linux:unknown:unknown";

// ── Gzip helpers ─────────────────────────────────────────────────────────────

fn gz(v: &impl Serialize) -> Result<Vec<u8>> {
    let json = serde_json::to_vec(v)?;
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(&json)?;
    Ok(enc.finish()?)
}

fn ungz<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T> {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut dec = GzDecoder::new(bytes);
        let mut out = Vec::new();
        dec.read_to_end(&mut out)?;
        serde_json::from_slice(&out).context("JSON parse error in gzip response")
    } else {
        serde_json::from_slice(bytes).context("JSON parse error in response")
    }
}

// ── Chunk format ─────────────────────────────────────────────────────────────
// Rows are tuples serialised as JSON arrays, matching Anki's wire format.

#[derive(Serialize, Deserialize, Default)]
struct Chunk {
    done: bool,
    #[serde(default)] revlog: Vec<Vec<Value>>,
    #[serde(default)] cards:  Vec<Vec<Value>>,
    #[serde(default)] notes:  Vec<Vec<Value>>,
}

// ── Sync client ───────────────────────────────────────────────────────────────

pub struct SyncClient {
    http: reqwest::Client,
    hkey: String,
}

impl SyncClient {
    /// Authenticate with AnkiWeb and return an authenticated client.
    ///
    /// AnkiWeb expects `p = sha256(username + ":" + password)` (hex-encoded).
    pub async fn login(username: &str, password: &str) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent("Anki/23.12.1 (Linux)")
            .timeout(std::time::Duration::from_secs(30))
            .build()?;

        let hashed_pw = {
            let mut h = Sha256::new();
            h.update(password.as_bytes());
            format!("{:x}", h.finalize())
        };

        #[derive(Serialize)]
        struct Creds<'a> { u: &'a str, p: &'a str }
        #[derive(Deserialize)]
        struct LoginResp { key: Option<String> }

        eprintln!("  POST {BASE}hostKey …");
        let resp = http.post(format!("{BASE}hostKey"))
            .header("Content-Type", "application/octet-stream")
            .body(gz(&Creds { u: username, p: &hashed_pw })?)
            .send().await?;

        let status = resp.status();
        let bytes = resp.bytes().await?;

        if !status.is_success() {
            let body = String::from_utf8_lossy(&bytes);
            bail!(
                "AnkiWeb login failed: HTTP {status}\nResponse: {}",
                if body.is_empty() { "(empty)" } else { &body }
            );
        }

        let login: LoginResp = ungz(&bytes).with_context(|| {
            format!(
                "Could not parse login response (HTTP {status}): {}",
                String::from_utf8_lossy(&bytes)
            )
        })?;
        let hkey = login.key.context("Login succeeded but AnkiWeb returned no host key")?;
        Ok(Self { http, hkey })
    }

    async fn post<Req, Resp>(&self, endpoint: &str, req: &Req) -> Result<Resp>
    where Req: Serialize, Resp: for<'de> Deserialize<'de>
    {
        let resp = self.http.post(format!("{BASE}{endpoint}"))
            .header("Content-Type", "application/octet-stream")
            .header("Authorization", format!("hkey {}", self.hkey))
            .body(gz(req)?)
            .send().await?;
        let status = resp.status();
        let bytes = resp.bytes().await?;
        if !status.is_success() {
            bail!(
                "AnkiWeb /{endpoint} failed: HTTP {status}\nResponse: {}",
                String::from_utf8_lossy(&bytes)
            );
        }
        ungz(&bytes).with_context(|| {
            format!(
                "Could not parse /{endpoint} response: {}",
                String::from_utf8_lossy(&bytes)
            )
        })
    }

    /// Perform an incremental sync against AnkiWeb.
    ///
    /// Sends local changes (cards/revlog/notes with usn=-1), pulls server
    /// changes (reviews done on other devices), and updates col.usn / col.ls.
    pub async fn sync(self, conn: &Connection) -> Result<SyncSummary> {
        // ── Local state ───────────────────────────────────────────────────
        let (local_mod, local_usn, local_scm): (i64, i64, i64) = conn.query_row(
            "SELECT mod, usn, scm FROM col LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;

        // ── Meta ──────────────────────────────────────────────────────────
        #[derive(Serialize)]
        struct MetaReq { v: u64, cv: &'static str }
        #[derive(Deserialize)]
        struct MetaResp {
            scm: i64,
            #[serde(rename = "mod")] server_mod: i64,
            usn: i64,
        }

        eprintln!("  meta…");
        let meta: MetaResp = self.post("meta", &MetaReq { v: SYNC_VER, cv: CLIENT_VER }).await?;

        if meta.scm != local_scm {
            bail!(
                "Schema mismatch (local scm={}, server scm={}). \
                 Open Anki desktop once to re-establish a sync baseline.",
                local_scm, meta.scm
            );
        }

        // ── Start ─────────────────────────────────────────────────────────
        #[derive(Serialize)]
        struct StartReq {
            #[serde(rename = "minUsn")] min_usn: i64,
            #[serde(rename = "lnewer")] l_newer: bool,
        }
        #[derive(Deserialize, Default)]
        struct StartResp {
            #[serde(rename = "fullSync", default)] full_sync: bool,
        }

        eprintln!("  start…");
        let l_newer = local_mod >= meta.server_mod;
        let start: StartResp = self.post("start", &StartReq {
            min_usn: local_usn,
            l_newer,
        }).await?;

        if start.full_sync {
            bail!(
                "Full sync required — your local collection has diverged from AnkiWeb. \
                 Open Anki desktop, sync there once to establish a baseline, then use ankr sync."
            );
        }

        // ── Step 4: Send graves (deleted notes/cards) — we have none ─────
        #[derive(Serialize)]
        struct ApplyGravesReq { chunk: GravesChunk }
        #[derive(Serialize, Default)]
        struct GravesChunk { notes: Vec<i64>, cards: Vec<i64>, decks: Vec<i64> }
        eprintln!("  applyGraves…");
        let _: Value = self.post("applyGraves", &ApplyGravesReq {
            chunk: GravesChunk::default(),
        }).await?;

        // ── Step 5: applyChanges — send/receive config changes ────────────
        // We have no config changes; server may respond with its own.
        #[derive(Serialize)]
        struct ApplyChangesReq { changes: NoChanges }
        #[derive(Serialize, Default)]
        struct NoChanges {
            models: Vec<Value>,
            decks: Vec<Value>,
            tags: Vec<Value>,
            #[serde(rename = "cconf")] deck_config: Vec<Value>,
        }
        eprintln!("  applyChanges…");
        let _: Value = self.post("applyChanges", &ApplyChangesReq {
            changes: NoChanges::default(),
        }).await?;

        // ── Step 6: Pull server chunks ────────────────────────────────────
        eprintln!("  pulling server chunks…");
        let mut pulled_cards = 0usize;
        loop {
            #[derive(Serialize)]
            struct Empty {}
            let server_chunk: Chunk = self.post("chunk", &Empty {}).await?;
            let done = server_chunk.done;
            pulled_cards += server_chunk.cards.len();
            apply_server_chunk(conn, server_chunk)?;
            if done { break; }
        }

        // ── Step 7: Push our local changes ───────────────────────────────
        eprintln!("  pushing local changes…");
        let local_chunk = collect_local_changes(conn)?;
        let pushed_cards   = local_chunk.cards.len();
        let pushed_revlog  = local_chunk.revlog.len();
        let pushed_notes   = local_chunk.notes.len();
        let _: Value = self.post("applyChunk", &local_chunk).await?;

        // ── Finish ────────────────────────────────────────────────────────
        #[derive(Serialize)]
        struct Empty {}
        #[derive(Deserialize)]
        struct FinishResp {
            #[serde(rename = "mod")] new_mod: i64,
            usn: i64,
            #[serde(default)] msg: String,
        }

        eprintln!("  finish…");
        let finish: FinishResp = self.post("finish", &Empty {}).await?;

        // Mark all locally-changed records as synced with the new USN.
        let new_usn = finish.usn;
        conn.execute("UPDATE cards  SET usn=?1 WHERE usn=-1", params![new_usn])?;
        conn.execute("UPDATE revlog SET usn=?1 WHERE usn=-1", params![new_usn])?;
        conn.execute("UPDATE notes  SET usn=?1 WHERE usn=-1", params![new_usn])?;
        conn.execute(
            "UPDATE col SET usn=?1, ls=?2, mod=?3",
            params![new_usn, finish.new_mod, finish.new_mod],
        )?;

        Ok(SyncSummary {
            pushed_cards,
            pushed_revlog,
            pushed_notes,
            pulled_cards,
            server_msg: finish.msg,
        })
    }
}

pub struct SyncSummary {
    pub pushed_cards:  usize,
    pub pushed_revlog: usize,
    pub pushed_notes:  usize,
    pub pulled_cards:  usize,
    pub server_msg:    String,
}

// ── Local change collection ───────────────────────────────────────────────────

fn collect_local_changes(conn: &Connection) -> Result<Chunk> {
    // Cards: id nid did ord mod [usn=-1] type queue due ivl factor reps lapses left odue odid flags data
    let cards = {
        let mut stmt = conn.prepare(
            "SELECT id,nid,did,ord,mod,type,queue,due,ivl,factor,reps,lapses,left,odue,odid,flags,data \
             FROM cards WHERE usn=-1"
        )?;
        let rows: Vec<_> = stmt.query_map([], |r| {
            Ok(vec![
                jnum(r.get::<_,i64>(0)?),   // id
                jnum(r.get::<_,i64>(1)?),   // nid
                jnum(r.get::<_,i64>(2)?),   // did
                jnum(r.get::<_,i64>(3)?),   // ord
                jnum(r.get::<_,i64>(4)?),   // mod
                jnum(-1i64),                 // usn (sent as -1; server assigns real USN)
                jnum(r.get::<_,i64>(5)?),   // type
                jnum(r.get::<_,i64>(6)?),   // queue
                jnum(r.get::<_,i64>(7)?),   // due
                jnum(r.get::<_,i64>(8)?),   // ivl
                jnum(r.get::<_,i64>(9)?),   // factor
                jnum(r.get::<_,i64>(10)?),  // reps
                jnum(r.get::<_,i64>(11)?),  // lapses
                jnum(r.get::<_,i64>(12)?),  // left
                jnum(r.get::<_,i64>(13)?),  // odue
                jnum(r.get::<_,i64>(14)?),  // odid
                jnum(r.get::<_,i64>(15)?),  // flags
                Value::String(r.get::<_,String>(16)?), // data
            ])
        })?.filter_map(|r| r.ok()).collect();
        rows
    };

    // Revlog: id cid [usn=-1] ease ivl lastIvl factor time type
    let revlog = {
        let mut stmt = conn.prepare(
            "SELECT id,cid,ease,ivl,lastIvl,factor,time,type FROM revlog WHERE usn=-1"
        )?;
        let rows: Vec<_> = stmt.query_map([], |r| {
            Ok(vec![
                jnum(r.get::<_,i64>(0)?),   // id
                jnum(r.get::<_,i64>(1)?),   // cid
                jnum(-1i64),                 // usn
                jnum(r.get::<_,i64>(2)?),   // ease
                jnum(r.get::<_,i64>(3)?),   // ivl
                jnum(r.get::<_,i64>(4)?),   // lastIvl
                jnum(r.get::<_,i64>(5)?),   // factor
                jnum(r.get::<_,i64>(6)?),   // time
                jnum(r.get::<_,i64>(7)?),   // type
            ])
        })?.filter_map(|r| r.ok()).collect();
        rows
    };

    // Notes: id guid mid mod [usn=-1] tags flds sfld csum flags data
    let notes = {
        let mut stmt = conn.prepare(
            "SELECT id,guid,mid,mod,tags,flds,sfld,csum,flags,data FROM notes WHERE usn=-1"
        )?;
        let rows: Vec<_> = stmt.query_map([], |r| {
            Ok(vec![
                jnum(r.get::<_,i64>(0)?),              // id
                Value::String(r.get::<_,String>(1)?),  // guid
                jnum(r.get::<_,i64>(2)?),              // mid
                jnum(r.get::<_,i64>(3)?),              // mod
                jnum(-1i64),                            // usn
                Value::String(r.get::<_,String>(4)?),  // tags
                Value::String(r.get::<_,String>(5)?),  // flds
                Value::String(r.get::<_,String>(6)?),  // sfld
                jnum(r.get::<_,i64>(7)?),              // csum
                jnum(r.get::<_,i64>(8)?),              // flags
                Value::String(r.get::<_,String>(9)?),  // data
            ])
        })?.filter_map(|r| r.ok()).collect();
        rows
    };

    Ok(Chunk { done: true, revlog, cards, notes })
}

fn jnum(n: i64) -> Value {
    Value::Number(n.into())
}

// ── Apply server chunk ────────────────────────────────────────────────────────

fn apply_server_chunk(conn: &Connection, chunk: Chunk) -> Result<()> {
    for row in &chunk.cards {
        if row.len() < 18 { continue; }
        let id = row[0].as_i64().unwrap_or(0);
        // Skip cards we have locally modified (usn=-1) — our version wins.
        let local_usn: Option<i64> = conn
            .query_row("SELECT usn FROM cards WHERE id=?1", params![id], |r| r.get(0))
            .ok();
        if local_usn == Some(-1) { continue; }

        conn.execute(
            "INSERT OR REPLACE INTO cards \
             (id,nid,did,ord,mod,usn,type,queue,due,ivl,factor,reps,lapses,left,odue,odid,flags,data) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",
            params![
                row[0].as_i64(),  row[1].as_i64(),  row[2].as_i64(),  row[3].as_i64(),
                row[4].as_i64(),  row[5].as_i64(),  row[6].as_i64(),  row[7].as_i64(),
                row[8].as_i64(),  row[9].as_i64(),  row[10].as_i64(), row[11].as_i64(),
                row[12].as_i64(), row[13].as_i64(), row[14].as_i64(), row[15].as_i64(),
                row[16].as_i64(), row[17].as_str().unwrap_or(""),
            ],
        )?;
    }

    for row in &chunk.revlog {
        if row.len() < 9 { continue; }
        // Revlog is append-only; ignore duplicates.
        let _ = conn.execute(
            "INSERT OR IGNORE INTO revlog (id,cid,usn,ease,ivl,lastIvl,factor,time,type) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                row[0].as_i64(), row[1].as_i64(), row[2].as_i64(), row[3].as_i64(),
                row[4].as_i64(), row[5].as_i64(), row[6].as_i64(), row[7].as_i64(),
                row[8].as_i64(),
            ],
        );
    }

    for row in &chunk.notes {
        if row.len() < 11 { continue; }
        let id = row[0].as_i64().unwrap_or(0);
        let local_usn: Option<i64> = conn
            .query_row("SELECT usn FROM notes WHERE id=?1", params![id], |r| r.get(0))
            .ok();
        if local_usn == Some(-1) { continue; }

        conn.execute(
            "INSERT OR REPLACE INTO notes \
             (id,guid,mid,mod,usn,tags,flds,sfld,csum,flags,data) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                row[0].as_i64(),
                row[1].as_str().unwrap_or(""),
                row[2].as_i64(), row[3].as_i64(), row[4].as_i64(),
                row[5].as_str().unwrap_or(""),
                row[6].as_str().unwrap_or(""),
                row[7].as_str().unwrap_or(""),
                row[8].as_i64(), row[9].as_i64(),
                row[10].as_str().unwrap_or(""),
            ],
        )?;
    }

    Ok(())
}
