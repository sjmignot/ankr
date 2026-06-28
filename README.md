# ankr

A terminal Anki client for reviewing flashcards, syncing with AnkiWeb, and creating cards — without the desktop app.

> **Warning: Direct database access**
>
> ankr reads from and writes directly to your Anki SQLite collection file (`collection.anki2`). A bug or unexpected failure could corrupt your collection. **Always keep Anki's automatic backups enabled** (Preferences → Backups), and **do not run ankr while Anki desktop is open** — concurrent access can cause data loss or corruption. ankr detects the Anki lock file and will warn you, but this is not a guarantee of safety.
>
> Use at your own risk. Back up your collection before first use.

## Features

- **TUI review** — review cloze and image cards with FSRS-based scheduling
- **Stats** — show due card counts per deck and exit
- **AnkiWeb sync** — push and pull changes without opening Anki desktop
- **Poem cards** — generate cloze cards from poems using the LPCG method
- **AI card creation** — generate cloze cards from arbitrary text via Claude

## Requirements

- Rust toolchain (to build from source)
- Anki 2.1.45 or later (for the modern database schema)
- Collection at the default path (`~/.local/share/Anki2/<profile>/collection.anki2` on Linux, `~/Library/Application Support/Anki2/<profile>/collection.anki2` on macOS)
- `ANTHROPIC_API_KEY` environment variable — required only for AI card creation

## Installation

```sh
cargo install ankr
```

## Usage

### Review (TUI)

```sh
ankr                        # review all due cards
ankr --deck "Geography"     # filter to a specific deck (substring match)
ankr --new-limit 10         # cap new cards per session (default: 20)
ankr --review-limit 100     # cap review cards per session (default: 200)
ankr --readonly             # preview mode — no writes to the database
ankr --db /path/to/collection.anki2  # use a non-default collection
```

### Stats

Print due card counts per deck and exit.

```sh
ankr stats
```

### Sync

Sync your collection with AnkiWeb. Credentials are resolved in order: CLI flag → environment variable → config file → interactive prompt.

```sh
ankr sync
ankr sync --username user@example.com --password secret
ANKIWEB_USER=user@example.com ANKIWEB_PASS=secret ankr sync
```

### Poem cards (LPCG method)

Create cloze cards from a poem using the [LPCG](https://controlaltbackspace.org/lpcg/) method.

```sh
# Interactive TUI editor
ankr poem --deck Poetry

# From a file
ankr poem sonnet18.txt --deck Poetry --tags "shakespeare sonnet"

# From stdin
cat sonnet18.txt | ankr poem --deck Poetry

# Stanza mode (one card per stanza instead of per line)
ankr poem sonnet18.txt --deck Poetry --stanza

# Preview without writing
ankr poem sonnet18.txt --deck Poetry --dry-run
```

### AI card creation

Generate cloze cards from arbitrary text using Claude. Requires `ANTHROPIC_API_KEY`.

```sh
export ANTHROPIC_API_KEY=sk-ant-...
ankr              # enter the TUI, then use the AI create screen
```

## Configuration

ankr stores optional configuration at `~/.config/ankr/config.toml` (Linux) or `~/Library/Application Support/ankr/config.toml` (macOS).

```toml
[sync]
username = "user@example.com"
password = "your-ankiweb-password"
```

> **Note:** The password is stored in plaintext. Set file permissions appropriately (`chmod 600 ~/.config/ankr/config.toml`).

You can also supply credentials via environment variables (`ANKIWEB_USER`, `ANKIWEB_PASS`) or be prompted interactively.

## License

MIT
