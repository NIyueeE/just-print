# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
# Run dev server (host/port from .env: HOST, PORT — default 127.0.0.1:3001)
uv run python backend/main.py

# Or with explicit override (for hot-reload during development)
uv run uvicorn backend.main:app --host 127.0.0.1 --port 3001 --reload

# Install dependencies
uv sync

# Start service via Podman Compose
podman-compose -f docker/compose.yaml up -d

# View logs
podman-compose -f docker/compose.yaml logs -f

# Rebuild and restart
podman-compose -f docker/compose.yaml up -d --build

# Or use just (run from docker/ directory)
cd docker && just up        # start
cd docker && just logs     # tail logs
cd docker && just update   # rebuild and restart
```

## Project Architecture

Stateless printing relay service — no database, no persistent storage, no user system. All state is in-memory per-session, destroyed after print or expiry.

### Directory Structure

```
├── backend/                # FastAPI backend
│   ├── main.py            # App entry, lifespan (session cleanup loop)
│   ├── config.py          # Config from env vars
│   ├── logging_config.py  # stdout logging (container-friendly)
│   ├── models.py          # Pydantic models
│   ├── routes.py          # All API endpoints
│   └── services/
│       ├── session.py     # In-memory session store (dict, 30-min TTL)
│       ├── auth.py        # Token verification (Bearer header or ?token=)
│       ├── file_handler.py # File validation, image→PDF, PDF merge
│       └── printer.py     # IPP printer interaction via pyipp
├── frontend/              # Vanilla JS single-page app
│   ├── index.html
│   ├── app.js
│   └── styles.css
├── docker/                # Deployment configs
│   ├── Dockerfile         # Multi-stage build with git clone
│   ├── compose.yaml       # Podman Compose (host networking, stateless)
│   └── justfile           # Shortcut commands
├── pyproject.toml          # Project metadata and dependencies
├── uv.lock                 # Lock file for reproducible installs
└── .env.example            # Environment variable template
```

### Key Design Decisions

- **Sessions**: `SessionData` dataclass in a global `dict[str, SessionData]`. 30-min inactivity TTL, background cleanup every 5 min.
- **PDF handling**: Files converted to PDF on upload, stored as bytes. Merged on-demand at preview/print time, not eagerly.
- **Concurrency**: `asyncio.Semaphore(2)` throttles file conversion. CPU-bound image processing runs via `asyncio.to_thread`.
- **IPP printing**: Uses `pyipp` library to build raw IPP binary packets, sent via HTTP POST (`application/ipp`). No CUPS dependency.
- **Printer capabilities**: Detected at runtime via IPP `Get-Printer-Attributes`. Unsupported options disabled in UI.
- **Docker build**: Multi-stage — clones repo, installs deps, then copies only runtime artifacts to a clean slim image.
- **Frontend**: Vanilla JS + Tailwind CSS (CDN). Printer status polling every 30s. Separate mobile/desktop preview.

### API Endpoints

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/` | No | Serve frontend HTML (cached in memory) |
| GET | `/health` | No | Health check |
| POST | `/auth` | No | Token authentication |
| POST | `/upload` | Token | File upload (JPG/PNG/PDF), rate-limited per IP |
| GET | `/preview.pdf` | Token | Merged PDF preview |
| GET | `/printer/status` | Token | Printer online/offline check |
| GET | `/printer/capabilities` | Token | Supported print options from printer |
| POST | `/print` | Token | Submit print job (session destroyed after) |
| POST | `/cancel` | Token | Clear session |

### Environment Config

All via env vars (`.env` loaded by `python-dotenv` in `config.py`). See `.env.example` for full list. Key vars: `PRINTER_IPP_URL`, `ACCESS_TOKEN`, `MAX_UPLOAD_MB` (default 50), `RATE_LIMIT_PER_IP` (default `5/minute`), `HOST` (default `127.0.0.1`), `PORT` (default `3001`).

### Deployment

- `docker/Dockerfile`: Multi-stage, `python:3.11-slim`, git clone from GitHub, `pip install .`
- `docker/compose.yaml`: `network_mode: host`, no volumes (stateless), all config via env vars
