# Aleph — Context Store for Your Desktop

One command, zero friction. Aleph runs silently in the background, captures window context via screenshots + AT-SPI, embeds everything with MiniLM (text) and SigLIP (vision), and serves a semantic search dashboard.

```bash
curl -fsSL https://raw.githubusercontent.com/Abdallah4Z/aleph/main/install.sh | bash
```

That's it. The script installs the binary, downloads models, and starts Aleph. No further interaction needed.

## Architecture

```
┌──────────────┐     ┌──────────────────┐     ┌──────────────┐
│  aleph       │────▶│  SQLite + Vectors │◀────│  aleph       │
│  daemon      │     │  (Embedded Vault) │     │  API         │
│  (xcap+ML)   │     └──────────────────┘     │  (Axum)      │
└──────┬───────┘                               └──────┬───────┘
       │                                              │
       ▼                                              ▼
  xcap captures                                   Dashboard
  MiniLM/SigLIP embed                             localhost:2198
```

## CLI Usage

```bash
aleph              # Start daemon + API
aleph daemon       # Just background capture
aleph api          # Just HTTP server
aleph stop         # systemctl --user stop aleph
aleph status       # Check if running
aleph config show  # Print config
aleph config set general.port 8080
```

## Configuration

`~/.config/aleph/config.toml` — all values adjustable from the dashboard at `http://localhost:2198/settings`.

Environment variables override config: `ALEPH_PORT`, `ALEPH_POLLING_INTERVAL`, `ALEPH_LOG_LEVEL`, etc.

## Data

`~/.local/share/aleph/` — SQLite database + ML model weights.

## How It Works

1. **xcap** polls the focused window every 2 seconds
2. On focus change: screenshot captured → encoded as PNG → passed to pipeline
3. **SigLIP** (ViT) embeds the screenshot into a 768-dim vector
4. **MiniLM** (BERT) embeds any accessible text into a 384-dim vector
5. **Dedup**: cosine similarity against last 5 vectors at 0.95 threshold
6. **SQLite** stores metadata + vector BLOBs
7. **Dashboard** queries via Axum API on port 2198

## Uninstall

```bash
systemctl --user stop aleph && systemctl --user disable aleph
rm -f ~/.local/bin/aleph ~/.config/systemd/user/aleph.service
rm -rf ~/.config/aleph ~/.local/share/aleph
```

## License

Abdallah Zain — All Rights Reserved.
