# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
# Run dev server (host/port from .env: HOST, PORT — default 127.0.0.1:3001)
uv run python backend/main.py

# Or with explicit override
uv run uvicorn backend.main:app --host 0.0.0.0 --port 3001 --reload

# Start all services via Podman Compose
podman-compose up -d

# View logs
podman-compose logs -f

# Rebuild and update
podman-compose up -d --build

# Restart service
podman-compose restart

# Using just (alternative to direct podman-compose)
just up        # start
just logs     # tail logs
just update   # rebuild and restart

# Install dependencies
uv sync
```

## Project Architecture

Stateless printing relay service — no database, no persistent storage, no user system. All state is in-memory per-session, destroyed after print or expiry.

### Backend Structure (`backend/`)

```
backend/
├── main.py              # FastAPI app entry, lifespan (session cleanup loop)
├── config.py            # Config from env vars via Config class
├── logging_config.py    # stdout logging setup (container-friendly)
├── models.py            # Pydantic models (AuthRequest, PrintRequest)
├── routes.py            # All API endpoints + route registration
└── services/
    ├── session.py       # In-memory session store (dict, 30-min TTL)
    ├── auth.py          # Token verification (Bearer header or ?token=)
    ├── file_handler.py  # File validation, image→PDF, PDF merge (asyncio.to_thread)
    └── printer.py       # IPP printer interaction via pyipp library
```

### Key Design Decisions

- **Sessions**: `SessionData` dataclass stored in a global `dict[str, SessionData]`. Sessions expire after 30 min of inactivity. Background cleanup task runs every 5 min.
- **PDF handling**: Files are converted to PDF on upload and stored as bytes in the session. PDFs are merged on-demand (at preview or print time), not eagerly.
- **Concurrency**: File conversion is throttled with `asyncio.Semaphore(2)`. CPU-bound image processing runs in thread pool via `asyncio.to_thread`.
- **IPP printing**: Uses `pyipp` library to build raw IPP binary packets and sends them via HTTP POST (`application/ipp`). No dependency on CUPS or system print commands.
- **Printer capabilities**: Detected at runtime via IPP `Get-Printer-Attributes`. Unsupported options are disabled in the UI.
- **Frontend**: Vanilla JS with Tailwind CSS (CDN). No framework. Printer status polling every 30s. Separate mobile/desktop preview layouts.

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
| POST | `/print` | Token | Submit print job, session destroyed after |
| POST | `/cancel` | Token | Clear session |

### Environment Config (`config.py` → `Config` class)

All via env vars (`.env` loaded by `python-dotenv`). Key vars: `PRINTER_IPP_URL`, `ACCESS_TOKEN`, `MAX_UPLOAD_MB` (default 50), `RATE_LIMIT_PER_IP` (default `5/minute`), IPP defaults (`IPP_DEFAULT_MEDIA`, `IPP_DEFAULT_QUALITY`, etc.).

### Frontend (`frontend/`)

- `index.html` — Single-page app with auth overlay + main UI
- `app.js` — All JS: auth, file upload (XHR with progress), PDF preview, printer capabilities UI, print settings
- `styles.css` — Custom variables, animations, responsive layout

### Deployment

- `Dockerfile`: `python:3.11-slim`, pip install, copy backend+frontend
- `compose.yaml`: `network_mode: host`, no volumes (stateless), all config via env vars
- Health check at `/health`
