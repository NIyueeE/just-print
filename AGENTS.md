# Repository Guidelines

Just Print is a single-container printing web service: a Rust (axum + tokio) backend
serving a Web API and the static frontend, with CUPS, headless LibreOffice, and fonts
built into the image. Uploaded documents are converted to PDF and handed to the
in-container CUPS queue for printing. Linux-only; the OCI image built from
`Containerfile` is the only delivery form (Docker and Podman compatible).

## Project Structure & Module Organization

- **Backend** (Rust 2024, axum + tokio + tower-http) under `src/`:
  - `src/main.rs` — entry point: config, tracing, background temp-file cleanup task, `axum::serve`.
  - `src/api/` — HTTP handlers: `mod.rs` (router + security headers + static-frontend fallback), `auth.rs` (Bearer middleware, constant-time compare), `middleware.rs` (request id/tracing span, metrics, request timeout), `files.rs` (streaming multipart upload + PDF preview + delete), `formats.rs` (supported extensions, single source of truth), `printers.rs` (printer list + IPP option catalog), `print.rs` (option encoding + idempotent `Print-Job` submit + failure reconciliation), `jobs.rs` (job list/status/cancel), `health.rs` (`/healthz`, `/readyz`, `/api/metrics`).
  - `src/cups/` — CUPS integration via standard IPP over HTTP (`ipp` crate): `mod.rs` (`CupsClient` with printer/job TTL caches + singleflight), `options.rs` (IPP attribute → option catalog + job-attribute encoding). No `lp`/`lpstat`/`lpoptions`/`ipptool` subprocesses, no PJL sessions, no device discovery; CUPS owns queues and scheduling.
  - `src/conversion.rs` — document → PDF: PDF magic-check passthrough, Markdown → styled HTML (`pulldown-cmark`, raw HTML escaped), all other formats via `soffice --headless` (2 concurrent slots, 2 min timeout).
  - `src/store.rs` — in-memory `FileStore` with reference counting (`FileGuard` RAII) and TTL cleanup; only converted temp PDFs are tracked, jobs live in CUPS.
  - `src/idempotency.rs` — `Idempotency-Key` store (fresh/replay/conflict/in-progress) with TTL and capacity.
  - `src/jobs.rs` — in-memory job metadata registry (`JobRecord`: file link, name, options) with TTL/capacity.
  - `src/metrics.rs` — dependency-free Prometheus registry (counters, gauges, fixed-bucket histograms).
  - `src/config.rs` — environment-variable config with fail-closed token validation (all limits are tunable).
  - `src/state.rs` — `AppState` (config, store, IPP client, idempotency store, job registry, metrics, upload/conversion semaphores) and `TempDir`.
  - `src/error.rs` — unified `AppError` → JSON envelope `{"error": {"code", "message"}}`, mapping CUPS failures to `503`/`504`/`502` and setting `Retry-After`.
  - `src/ids.rs` — CSPRNG id generator (UUID v4, 32 hex chars).
- **Frontend**: Preact + Vite + TypeScript under `frontend/`; reusable UI in `frontend/src/components/` (`TokenGate` / `Uploader` / `PrinterPanel` / `JobList` / `FlowSteps`), API types and calls in `frontend/src/api.ts`, shared state in `frontend/src/state.ts`, design tokens in `frontend/src/styles/tokens.css`, per-component CSS next to each component, icons in `icons.tsx`. ESLint (`jsx-a11y`) + Prettier + Vitest. `bun run build` runs `tsc -b && vite build`; `bun run lint` and `bun run test:run` are part of `just frontend`.
- **Runtime container**: `Containerfile` (multi-stage: bun frontend build → rust release build → Debian bookworm-slim with CUPS, LibreOffice, Ghostscript, poppler-utils, CJK/Symbola fonts) and `container/entrypoint.sh` (starts CUPS; optional cups-pdf debug printer, Avahi/IPP discovery, USB printer auto-add with model-matched vendor PPDs via the bundled HPLIP driver, and `JUST_PRINT_USB_OPTIONS` for installable PPD options such as a duplexer; helper logic is covered by `container/entrypoint.test.sh`).
- **Docs and deployment**: API contract in `docs/api.md`, guides in `docs/architecture.md`, `docs/development.md`, `docs/deployment.md`; Compose and Quadlet examples in `examples/`; CI and release workflows in `.github/workflows/`.

## Build, Test, and Development Commands

Requires `just`, Rust, and Bun; container builds need Podman or Docker.

- `just check` – full backend + frontend validation (the CI job).
- `just backend` – `cargo fmt --check`, clippy `--all-targets --all-features -D warnings`, `cargo test --all-features`.
- `just frontend` – frozen-lockfile install (`bun install --frozen-lockfile`) + `bun run typecheck` + `bun run lint` + `bun run test:run` + `bun run build`.
- `just container` – build local image `ghcr.io/niyueee/just-print:local` (auto-selects podman or docker).
- `just debug` – build and run a foreground container with `JUST_PRINT_CUPS_PDF=1`; requires `JUST_PRINT_TOKEN` set first.
- `just init-hooks` – `git config core.hooksPath .githooks` so the pre-commit hook runs `just check` on every commit.
- `cd frontend && bun run dev` – Vite dev server proxying `/api` to `http://localhost:8080`; run the backend (`cargo run` with `JUST_PRINT_TOKEN` set, or `just debug`) for API work.
- Visual UI walk-through without CUPS/LibreOffice on the host (mock API + headless Chromium/Playwright + imageread) is documented in `docs/development.md` under "前端视觉检查（无容器环境）".

## Coding Style & Naming Conventions

- **Rust lints** are declared in `Cargo.toml` (`[lints.rust]` / `[lints.clippy]`): rust denies `unsafe_code`, `missing_docs`, `non_ascii_idents`, `warnings`; clippy denies `all`, `pedantic`, `cargo`, `unwrap_used`, `expect_used`, `panic`, `indexing_slicing`. Prefer `Result`/`?`; never `unwrap`, `expect`, `panic`, or index directly.
- **Names**: snake_case modules and test functions; every public item needs a doc comment (`missing_docs` is deny).
- **Language**: code comments, doc comments, and docs under `docs/` are written in Chinese; commit subjects are English.
- **Frontend**: `tsc -b` type-checking; Preact functional components in PascalCase `.tsx` files, camelCase functions/variables, styles in `app.css` / `index.css`.

## Testing Guidelines

- Backend tests are inline `#[cfg(test)] mod tests` blocks using `#[test]` / `#[tokio::test]`, focused on parsers and pure logic without a live CUPS or LibreOffice.
- Name tests descriptively in snake_case, e.g. `parses_lpstat_printer_blocks`, `markdown_escapes_raw_html`, `rejects_unknown_and_invalid_option_values`.
- Run them with `cargo test --all-features` or `just backend`.
- Frontend has Vitest component/unit tests plus ESLint a11y rules; `just frontend` type-checks, lints, tests and builds. CI adds a container image build with an end-to-end smoke test (401 without token, readiness, upload → preview → print → completed on the CUPS-PDF printer, idempotent replay, job list/cancel, file delete) plus dependency audit, image vulnerability scan and `docker compose config` validation.

## Commit & Pull Request Guidelines

- Use Conventional Commits (`feat:`, `fix:`, `docs:`, `ci:`); keep subjects imperative and concise (English), and explain why in the body.
- Run `just check` before opening a PR; the pre-commit hook enforces it on every commit once `just init-hooks` has been run.
- Summarize the change, link related issues, describe testing, and add screenshots for UI changes. Avoid unrelated edits.

## Security & Configuration

- `JUST_PRINT_TOKEN` is required; the service fails closed (refuses to start) when unset or empty. All `/api/*` routes except `/healthz` require `Authorization: Bearer <token>`, compared in constant time (`subtle`).
- Never commit tokens, credentials, or machine-specific paths; environment variables are documented in `docs/deployment.md` and `README.md`.
- The container is the only delivery form; the backend serves plain HTTP and TLS is terminated by a gateway/reverse proxy.
- `POST /api/print` accepts the standard `Idempotency-Key` header; the server replays the stored response for the same key+payload and reconciles a lost `Print-Job` response through the IPP `job-name` marker to avoid duplicate prints.
- Uploaded files live in `/tmp/just-print` (refcounted, 30 min TTL, cleaned every minute) and are lost on restart; CUPS keeps its own spool. Job metadata and idempotency keys are in memory only.
