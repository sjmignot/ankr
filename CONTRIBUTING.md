# Contributing to ankr

## Prerequisites

- Rust toolchain (stable, via `rustup`)
- An Anki collection at the default path (`~/.local/share/Anki2/<Profile>/collection.anki2`)

> **Warning:** ankr reads from and writes directly to the Anki SQLite database. Use `--readonly` when developing to avoid accidental data modification. Always keep Anki backups enabled.

## Build & Run

```sh
cargo build                     # debug build
cargo run                       # launch the TUI against your real collection
cargo run -- --readonly         # safe read-only mode
cargo run -- stats              # print deck stats to stdout
cargo run -- --db /path/to/collection.anki2   # specify collection path
```

## Project Structure

```
src/
├── main.rs              # CLI entry point (clap subcommands: default TUI, stats, sync, poem)
├── cli.rs               # Clap argument / command definitions
├── config.rs            # Config file (~/.config/ankr/config.toml): sync credentials
├── error.rs             # AnkrError enum + Result type alias
├── models.rs            # Core data models: Card, Note, Deck, CardState, ReviewResult, …
│
├── db/
│   ├── mod.rs           # DbConn wrapper around rusqlite::Connection; WAL mode setup
│   ├── queries.rs       # All SQL: reads (decks, cards, notes, notetypes) and writes
│   │                    #   (write_review, insert_note, get_or_create_deck_path)
│   └── lock.rs          # Detects whether Anki desktop holds the collection lock
│
├── scheduler/
│   └── fsrs.rs          # FSRS v5 scheduling via the rs-fsrs crate; produces CardState
│
├── review/
│   └── queue.rs         # Per-session ReviewQueue: learning → due → new priority order
│
├── render/
│   ├── cloze.rs         # Cloze deletion rendering: render_question / render_answer
│   ├── html.rs          # html2text wrapper: strips HTML tags, extracts <img> src attrs
│   ├── image.rs         # Terminal image rendering (quadrant-block Unicode art)
│   └── poem.rs          # LPCG poem card generation (line-by-line cloze sequence)
│
├── tui/
│   ├── mod.rs           # Terminal setup, main event loop, screen state machine
│   ├── events.rs        # (reserved)
│   └── screens/
│       ├── deck_select.rs   # Deck list with collapsible hierarchy (l/h to expand/collapse)
│       ├── review.rs        # Card review: question → typed answer → reveal → rating (1-4)
│       ├── create.rs        # Manual card creation form
│       ├── poem_create.rs   # LPCG poem card creation (paste poem, choose granularity)
│       ├── ai_create.rs     # AI card generation via Claude API
│       └── done.rs          # Session summary screen
│
├── sync.rs              # AnkiWeb sync protocol v11: zstd-compressed JSON over HTTPS
│                        #   Implements: hostKey → meta → applyGraves → chunk → applyChunk
│                        #   → finish; stamps USN on local changes before push
│
└── ai/
    └── mod.rs           # Claude API client for AI card generation (ANTHROPIC_API_KEY)
```

## Database Schema (key tables)

| Table       | Purpose |
|-------------|---------|
| `col`       | Single-row collection metadata: creation time (`crt`), USN, sync state |
| `decks`     | Deck definitions stored as JSON in `col.decks` (parsed in `queries.rs`) |
| `notetypes` | Note type definitions: field names, template front/back, cloze vs standard |
| `notes`     | Note content: `flds` (fields separated by `\x1f`), `tags`, `mid` (notetype id) |
| `cards`     | Card scheduling: `type` (0=New,1=Learning,2=Review,3=Relearning), `due`, `ivl`, `factor`, `data` (FSRS JSON) |
| `revlog`    | Immutable review history log |
| `graves`    | Deleted object ids pending sync |

## Sync Protocol Notes

The sync implementation in `sync.rs` uses Anki's v11 protocol:

1. All requests are zstd-compressed JSON with an `anki-sync` header
2. `due` field semantics differ by card type: **learning cards** store a Unix timestamp; **review cards** store a day offset from `col.crt`
3. USN (Update Sequence Number) tracks which objects need to be pushed; local unsent changes have `usn = -1`

## Adding a New Screen

1. Create `src/tui/screens/<name>.rs` with a screen struct and an action enum
2. Export it from `src/tui/screens/mod.rs`
3. Add a variant to the `Screen` enum in `src/tui/mod.rs`
4. Handle transitions to/from the new screen in `run_app`

## Configuration

`~/.config/ankr/config.toml`:
```toml
[sync]
username = "you@example.com"
password = "yourpassword"     # stored in plaintext
```

`ANTHROPIC_API_KEY` environment variable is required for AI card generation (`ankr` subcommand via `[a]` in the review screen).
