# Context-Store (Engramd MVP)

A Local-First, Agentic Context Retrieval System built in Rust under the Nomeda Lab umbrella.

## Architecture

```
┌─────────────────┐     ┌──────────────────┐     ┌─────────────────┐
│  context-daemon │────▶│  SQLite + Vectors │◀────│  context-api    │
│  (OS Spy)       │     │  (Embedded Vault) │     │  (Retrieval)    │
└─────────────────┘     └──────────────────┘     └─────────────────┘
         │                                               │
         ▼                                               ▼
   AT-SPI / Mock                                    n8n + Ollama
   xcap + SigLIP                                    (Agentic Flow)
```

### Crates

- **`context-core`** — Shared traits, models, SQLite-backed vector store, deduplication logic, and Candle-based encoders (MiniLM + SigLIP placeholder).
- **`context-daemon`** — Background daemon. Listens to window focus events (AT-SPI on Linux, Mock mode for headless), hashes text, generates embeddings, and stores context with 95% deduplication.
- **`context-api`** — Axum HTTP API on port `2198`. Accepts natural language queries, embeds them, and performs dual-vector KNN search against the vault.

## Quick Start

### 1. Build

```bash
export PATH="/tmp/protoc/bin:$PATH"  # if protoc is not system-installed
cargo build --release
```

### 2. Run the Daemon (Mock Mode for Headless / CI)

```bash
MOCK_MODE=1 ./target/release/context-daemon
```

In production on Ubuntu Desktop:

```bash
./target/release/context-daemon
```

### 3. Run the API

```bash
./target/release/context-api
```

The API listens on `http://127.0.0.1:2198`.

### 4. Query

```bash
curl -X POST http://127.0.0.1:2198/query \
  -H "Content-Type: application/json" \
  -d '{"question":"What was I reading about Rust?","top_k":3}'
```

### 5. Docker Stack (n8n + Ollama)

```bash
docker-compose up -d
```

- n8n: http://localhost:5678 (admin / engramd123)
- Ollama: http://localhost:11434

The default model pulled is `qwen2.5:0.5b`. To use another model:

```bash
docker exec -it engramd-ollama ollama pull qwen2.5:0.5b
```

## Data Directory

All embedded data lives in `./data/`:

- `metadata.db` — SQLite database with event metadata and vector blobs
- `models/` — Downloaded ML weights (MiniLM, SigLIP)

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `MOCK_MODE` | `0` | Use `dummy_data.json` instead of AT-SPI |
| `CONTEXT_API_PORT` | `2198` | Port for the Axum API server |
| `RUST_LOG` | — | Set to `info` or `debug` for tracing |

## Deduplication Logic

Before inserting any new vector:

1. Compute cosine similarity against the last 5 stored vectors.
2. If similarity > `0.95`, update `end_time` of the existing event only.
3. If similarity <= `0.95`, insert a new metadata row + vector blob.

This keeps the database tiny and query latency sub-50ms for MVP-scale datasets.

## n8n Workflow (Agentic Retrieval)

1. **Chat Trigger** — User asks a question.
2. **HTTP Request** — n8n POSTs to `http://host.docker.internal:2198/query`.
3. **Code Node** — Formats the returned `context` + `sources`.
4. **Ollama Chat Model** — Sends context + question to the local LLM.
5. **Respond to Webhook** — Returns the synthesized answer.

An example workflow JSON is provided in `tests/n8n_workflow.json`.

## Production Notes

- **AT-SPI**: The daemon auto-detects AT-SPI via D-Bus. If unavailable, it gracefully falls back to the stub.
- **MiniLM**: Place `config.json`, `tokenizer.json`, and `model.safetensors` into `./data/models/all-MiniLM-L6-v2/`. The dummy encoder is used as a fallback.
- **SigLIP**: Place vision model weights into `./data/models/siglip/`. Currently returns deterministic hash-based embeddings for MVP.
- **LanceDB**: The core crate includes an optional `lancedb-store` feature for migrating to LanceDB when the dataset grows beyond ~10k vectors.

## License

Abdallah Zain — All Rights Reserved.
