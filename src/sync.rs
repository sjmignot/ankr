use anyhow::{bail, Context, Result};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Read;

const BASE: &str = "https://sync.ankiweb.net/";
const SYNC_VER: u8 = 11;
// Exact values captured via mitmproxy from real Anki 25.09.4 client.
// Server validates the format; using our own identifiers may cause 400.
const CLIENT_VER_HEADER: &str = "25.09.4,d52ca669,linux";
const CLIENT_VER_BODY:   &str = "anki,25.09.4 (d52ca669),lin";

// ── Zstd helpers ──────────────────────────────────────────────────────────────

fn zstd_enc(v: &impl Serialize) -> Result<Vec<u8>> {
    let json = serde_json::to_vec(v)?;
    Ok(zstd::encode_all(json.as_slice(), 0)?)
}

fn zstd_dec<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T> {
    // Server may respond uncompressed if the payload is tiny.
    if bytes.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        let mut dec = zstd::Decoder::new(bytes)?;
        let mut out = Vec::new();
        dec.read_to_end(&mut out)?;
        serde_json::from_slice(&out).context("JSON parse error in zstd response")
    } else {
        serde_json::from_slice(bytes).context("JSON parse error in response")
    }
}

// ── Sync header (sent on every request) ──────────────────────────────────────

#[derive(Serialize)]
struct SyncHeader<'a> {
    v: u8,
    k: &'a str,
    c: &'static str,
    s: &'a str,
}

// ── Graves format ─────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Default)]
struct Graves {
    #[serde(default)] cards: Vec<i64>,
    #[serde(default)] notes: Vec<i64>,
    #[serde(default)] decks: Vec<i64>,
}

// ── Chunk format ──────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Default)]
struct Chunk {
    done: bool,
    #[serde(default)] revlog: Vec<Vec<Value>>,
    #[serde(default)] cards:  Vec<Vec<Value>>,
    #[serde(default)] notes:  Vec<Vec<Value>>,
}

// ── Sync client ───────────────────────────────────────────────────────────────

pub struct SyncClient {
    http:     reqwest::Client,
    hkey:     String,
    session:  String,
    endpoint: String,  // base URL, may be redirected to sync4.ankiweb.net etc.
}

impl SyncClient {
    pub async fn login(username: &str, password: &str) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent("Anki/23.12.1 (Linux)")
            .timeout(std::time::Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;

        let session = simple_session_id();

        #[derive(Serialize)]
        struct Creds<'a> { u: &'a str, p: &'a str }
        #[derive(Deserialize)]
        struct LoginResp { key: Option<String> }

        eprintln!("  POST {BASE}sync/hostKey …");
        let header = SyncHeader { v: SYNC_VER, k: "", c: CLIENT_VER_HEADER, s: &session };
        let resp = http.post(format!("{BASE}sync/hostKey"))
            .header("Content-Type", "application/octet-stream")
            .header("anki-sync", serde_json::to_string(&header)?)
            .body(zstd_enc(&Creds { u: username, p: password })?)
            .send().await?;

        let status = resp.status();
        let bytes = resp.bytes().await?;

        if !status.is_success() {
            bail!(
                "AnkiWeb login failed: HTTP {status}\nResponse: {}",
                String::from_utf8_lossy(&bytes)
            );
        }

        let login: LoginResp = zstd_dec(&bytes)?;
        let hkey = login.key.context("Login succeeded but AnkiWeb returned no host key")?;
        Ok(Self { http, hkey, session, endpoint: BASE.to_string() })
    }

    async fn post<Req, Resp>(&self, endpoint: &str, req: &Req) -> Result<Resp>
    where Req: Serialize, Resp: for<'de> Deserialize<'de>
    {
        let body_json = serde_json::to_vec(req)?;
        let header = SyncHeader { v: SYNC_VER, k: &self.hkey, c: CLIENT_VER_HEADER, s: &self.session };
        let resp = self.http.post(format!("{}sync/{endpoint}", self.endpoint))
            .header("Content-Type", "application/octet-stream")
            .header("anki-sync", serde_json::to_string(&header)?)
            .body(zstd::encode_all(body_json.as_slice(), 0)?)
            .send().await?;
        let status = resp.status();
        let bytes = resp.bytes().await?;
        if !status.is_success() {
            bail!(
                "AnkiWeb /{endpoint} failed: HTTP {status}\nResponse: {}",
                String::from_utf8_lossy(&bytes)
            );
        }
        let decoded: Resp = zstd_dec(&bytes).with_context(|| {
            format!(
                "Could not parse /{endpoint} response: {}",
                String::from_utf8_lossy(&bytes)
            )
        })?;
        Ok(decoded)
    }

    pub async fn sync(mut self, conn: &Connection) -> Result<SyncSummary> {
        let (local_mod, local_usn, local_scm): (i64, i64, i64) = conn.query_row(
            "SELECT mod, usn, scm FROM col LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;

        // ── Meta (reqwest follows the 308 to the assigned sync server) ───────
        // AnkiWeb routes each account to syncN.ankiweb.net via 308 redirect.
        // We let reqwest follow it, then read resp.url() to update our endpoint.
        eprintln!("  meta…");
        #[derive(Serialize)]
        struct MetaReq { v: u8, cv: &'static str }
        #[derive(Deserialize)]
        struct MetaResp {
            scm: i64,
            #[serde(rename = "mod")] server_mod: i64,
            usn: i64,
        }

        let meta: MetaResp = {
            let send_meta = |endpoint: &str, hkey: &str, session: &str| {
                let header = SyncHeader { v: SYNC_VER, k: hkey, c: CLIENT_VER_HEADER, s: session };
                self.http
                    .post(format!("{endpoint}sync/meta"))
                    .header("Content-Type", "application/octet-stream")
                    .header("anki-sync", serde_json::to_string(&header).unwrap())
                    .body(zstd_enc(&MetaReq { v: SYNC_VER, cv: CLIENT_VER_BODY }).unwrap())
            };

            // First attempt — may 308-redirect to the assigned sync server.
            let r1 = send_meta(&self.endpoint, &self.hkey, &self.session).send().await?;
            let r1_status = r1.status();

            let resp = if r1_status.as_u16() == 308 {
                // 308 Location is the new BASE (e.g. https://sync4.ankiweb.net/).
                // Append /sync/meta and resend — do not follow the redirect blindly.
                let loc = r1.headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .context("308 without Location header")?
                    .to_string();
                let new_base = if loc.ends_with('/') { loc } else { format!("{loc}/") };
                eprintln!("  → assigned to {new_base}");
                self.endpoint = new_base;
                send_meta(&self.endpoint, &self.hkey, &self.session).send().await?
            } else {
                r1
            };

            let status = resp.status();
            let bytes = resp.bytes().await?;
            if !status.is_success() {
                bail!("AnkiWeb /meta failed: HTTP {status}\nResponse: {}", String::from_utf8_lossy(&bytes));
            }
            zstd_dec(&bytes).context("parsing meta response")?
        };

        if meta.scm != local_scm {
            bail!(
                "Schema mismatch (local scm={}, server scm={}). \
                 Open Anki desktop once to re-establish a sync baseline.",
                local_scm, meta.scm
            );
        }

        // ── Start ─────────────────────────────────────────────────────────
        eprintln!("  start…");
        // v11 protocol: graves are included inline in the start body (null = no local graves).
        #[derive(Serialize)]
        struct StartReq {
            #[serde(rename = "minUsn")] min_usn: i64,
            #[serde(rename = "lnewer")] l_newer: bool,
            graves: Option<()>,
        }
        #[derive(Deserialize, Default)]
        struct StartResp {
            #[serde(rename = "fullSync", default)] full_sync: bool,
            #[serde(default)] graves: Graves,
        }
        let l_newer = local_mod >= meta.server_mod;
        let start: StartResp = self.post("start", &StartReq { min_usn: local_usn, l_newer, graves: None }).await?;

        if start.full_sync {
            bail!(
                "Full sync required — your local collection has diverged from AnkiWeb. \
                 Open Anki desktop, sync there once to establish a baseline, then use ankr sync."
            );
        }

        // ── applyGraves ───────────────────────────────────────────────────
        // Protocol: client applies server's graves locally, then echoes them back
        // to the server wrapped in "chunk" (not "graves") as confirmation.
        eprintln!("  applyGraves…");
        apply_server_graves(conn, &start.graves)?;
        #[derive(Serialize)]
        struct ApplyGravesReq { chunk: Graves }
        let _: Value = self.post("applyGraves", &ApplyGravesReq { chunk: start.graves }).await?;

        // ── applyChanges ──────────────────────────────────────────────────
        eprintln!("  applyChanges…");
        // v11 format (from rslib/src/sync/collection/changes.rs):
        //   models = Vec<Notetype>
        //   decks  = { "decks": Vec<Deck>, "config": Vec<DeckConfig> }  ← nested object, NOT array
        //   tags   = Vec<String>
        //   conf   = Option<HashMap>  (collection config, optional)
        //   crt    = Option<i64>      (creation stamp, optional)
        #[derive(Serialize)]
        struct ApplyChangesReq { changes: UnchunkedChanges }
        #[derive(Serialize)]
        struct UnchunkedChanges {
            models: Vec<Value>,
            decks:  DecksAndConfig,
            tags:   Vec<Value>,
        }
        #[derive(Serialize)]
        struct DecksAndConfig {
            decks:  Vec<Value>,
            config: Vec<Value>,
        }
        let _: Value = self.post("applyChanges", &ApplyChangesReq {
            changes: UnchunkedChanges {
                models: vec![],
                decks:  DecksAndConfig { decks: vec![], config: vec![] },
                tags:   vec![],
            },
        }).await?;

        // ── Collect local changes BEFORE pulling server chunks ────────────
        // Server chunks arrive with usn=-1; if we collected after, we'd
        // reflect the server's own cards back at it in applyChunk.
        eprintln!("  collecting local changes…");
        let local_chunk = collect_local_changes(conn)?;
        let pushed_cards  = local_chunk.cards.len();
        let pushed_revlog = local_chunk.revlog.len();
        let pushed_notes  = local_chunk.notes.len();

        // ── Pull server chunks ────────────────────────────────────────────
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
        // Server expects {"chunk": {...}} wrapper (ApplyChunkRequest), not a bare Chunk.
        #[derive(Serialize)]
        struct ApplyChunkReq { chunk: Chunk }
        let _: Value = self.post("applyChunk", &ApplyChunkReq { chunk: local_chunk }).await?;

        // ── Finish ────────────────────────────────────────────────────────
        eprintln!("  finish…");
        #[derive(Serialize)]
        struct Empty {}
        // v11: finish returns the new server mod as a bare JSON integer (milliseconds).
        // col.ls and col.mod are stored in seconds; convert so touch_col comparisons work.
        let new_mod_ms: i64 = self.post("finish", &Empty {}).await?;
        let new_mod_secs = if new_mod_ms > 10_000_000_000 { new_mod_ms / 1000 } else { new_mod_ms };
        let new_usn = meta.usn;

        conn.execute("UPDATE cards  SET usn=?1 WHERE usn=-1", params![new_usn])?;
        conn.execute("UPDATE revlog SET usn=?1 WHERE usn=-1", params![new_usn])?;
        conn.execute("UPDATE notes  SET usn=?1 WHERE usn=-1", params![new_usn])?;
        conn.execute("UPDATE graves SET usn=?1 WHERE usn=-1", params![new_usn])?;
        conn.execute(
            "UPDATE col SET usn=?1, ls=?2, mod=?3",
            params![new_usn, new_mod_secs, new_mod_secs],
        )?;

        Ok(SyncSummary {
            pushed_cards,
            pushed_revlog,
            pushed_notes,
            pulled_cards,
            server_msg: String::new(),
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

// ── Session ID ────────────────────────────────────────────────────────────────

fn simple_session_id() -> String {
    let table = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as usize;
    (0..8).map(|i| table[(n.wrapping_shr(i * 4)) % table.len()] as char).collect()
}

// ── Local change collection ───────────────────────────────────────────────────

fn collect_local_changes(conn: &Connection) -> Result<Chunk> {
    let cards = {
        let mut stmt = conn.prepare(
            "SELECT id,nid,did,ord,mod,type,queue,due,ivl,factor,reps,lapses,left,odue,odid,flags,data \
             FROM cards WHERE usn=-1"
        )?;
        let rows: Vec<_> = stmt.query_map([], |r| {
            Ok(vec![
                jnum(r.get::<_,i64>(0)?),
                jnum(r.get::<_,i64>(1)?),
                jnum(r.get::<_,i64>(2)?),
                jnum(r.get::<_,i64>(3)?),
                jnum(r.get::<_,i64>(4)?),
                jnum(-1i64),
                jnum(r.get::<_,i64>(5)?),
                jnum(r.get::<_,i64>(6)?),
                jnum(r.get::<_,i64>(7)?),
                jnum(r.get::<_,i64>(8)?),
                jnum(r.get::<_,i64>(9)?),
                jnum(r.get::<_,i64>(10)?),
                jnum(r.get::<_,i64>(11)?),
                jnum(r.get::<_,i64>(12)?),
                jnum(r.get::<_,i64>(13)?),
                jnum(r.get::<_,i64>(14)?),
                jnum(r.get::<_,i64>(15)?),
                Value::String(r.get::<_,String>(16)?),
            ])
        })?.filter_map(|r| r.ok()).collect();
        rows
    };

    let revlog = {
        let mut stmt = conn.prepare(
            "SELECT id,cid,ease,ivl,lastIvl,factor,time,type FROM revlog WHERE usn=-1"
        )?;
        let rows: Vec<_> = stmt.query_map([], |r| {
            Ok(vec![
                jnum(r.get::<_,i64>(0)?),
                jnum(r.get::<_,i64>(1)?),
                jnum(-1i64),
                jnum(r.get::<_,i64>(2)?),
                jnum(r.get::<_,i64>(3)?),
                jnum(r.get::<_,i64>(4)?),
                jnum(r.get::<_,i64>(5)?),
                jnum(r.get::<_,i64>(6)?),
                jnum(r.get::<_,i64>(7)?),
            ])
        })?.filter_map(|r| r.ok()).collect();
        rows
    };

    let notes = {
        // v11: sfld and csum are always empty strings in NoteEntry (server recomputes them).
        let mut stmt = conn.prepare(
            "SELECT id,guid,mid,mod,tags,flds,flags,data FROM notes WHERE usn=-1"
        )?;
        let rows: Vec<_> = stmt.query_map([], |r| {
            Ok(vec![
                jnum(r.get::<_,i64>(0)?),
                Value::String(r.get::<_,String>(1)?),
                jnum(r.get::<_,i64>(2)?),
                jnum(r.get::<_,i64>(3)?),
                jnum(-1i64),
                Value::String(r.get::<_,String>(4)?),
                Value::String(r.get::<_,String>(5)?),
                Value::String(String::new()),           // sfld: always "" in v11
                Value::String(String::new()),           // csum: always "" in v11
                jnum(r.get::<_,i64>(6)?),
                Value::String(r.get::<_,String>(7)?),
            ])
        })?.filter_map(|r| r.ok()).collect();
        rows
    };

    Ok(Chunk { done: true, revlog, cards, notes })
}

fn jnum(n: i64) -> Value {
    Value::Number(n.into())
}

// ── Graves helpers ────────────────────────────────────────────────────────────

fn apply_server_graves(conn: &Connection, graves: &Graves) -> Result<()> {
    for id in &graves.cards {
        conn.execute("DELETE FROM cards WHERE id=?1", params![id])?;
    }
    for id in &graves.notes {
        conn.execute("DELETE FROM notes WHERE id=?1", params![id])?;
    }
    for id in &graves.decks {
        conn.execute("DELETE FROM decks WHERE id=?1", params![id])?;
    }
    Ok(())
}

// ── Apply server chunk ────────────────────────────────────────────────────────

fn apply_server_chunk(conn: &Connection, chunk: Chunk) -> Result<()> {
    for row in &chunk.cards {
        if row.len() < 18 { continue; }
        let id = row[0].as_i64().unwrap_or(0);
        let local_usn: Option<i64> = conn
            .query_row("SELECT usn FROM cards WHERE id=?1", params![id], |r| r.get(0))
            .ok();
        if local_usn == Some(-1) { continue; }
        conn.execute(
            "INSERT OR REPLACE INTO cards \
             (id,nid,did,ord,mod,usn,type,queue,due,ivl,factor,reps,lapses,left,odue,odid,flags,data) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",
            params![
                row[0].as_i64(), row[1].as_i64(), row[2].as_i64(), row[3].as_i64(),
                row[4].as_i64(), row[5].as_i64(), row[6].as_i64(), row[7].as_i64(),
                row[8].as_i64(), row[9].as_i64(), row[10].as_i64(), row[11].as_i64(),
                row[12].as_i64(), row[13].as_i64(), row[14].as_i64(), row[15].as_i64(),
                row[16].as_i64(), row[17].as_str().unwrap_or(""),
            ],
        )?;
    }

    for row in &chunk.revlog {
        if row.len() < 9 { continue; }
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
        // v11: server sends sfld="" and csum="" (empty strings); client recomputes.
        // csum has NOT NULL constraint so we store 0; Anki desktop fixes it on next open.
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
                row[7].as_str().unwrap_or(""),          // sfld: "" is fine
                row[8].as_i64().unwrap_or(0),           // csum: "" → 0
                row[9].as_i64().unwrap_or(0),           // flags
                row[10].as_str().unwrap_or(""),         // data
            ],
        )?;
    }

    Ok(())
}
