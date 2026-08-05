# Repository Guidelines

## Project Structure & Module Organization

- **Backend**: Rust 2024 with axum and tokio under `src/`. Handlers live in `src/api/`, PJL logic in `src/pjl/`, discovery and per-printer workers in `src/registry.rs` and `src/workers.rs`, conversion in `src/conversion.rs`, and configuration in `src/config.rs`.
- **Frontend**: Preact + Vite + TypeScript under `frontend/`; reusable UI lives in `frontend/src/components/`, API types and calls in `frontend/src/api.ts`.
- **Docs and deployment**: API contract in `docs/api.md`, Compose and Quadlet examples in `examples/`, container build in `Containerfile`, CI and release workflows in `.github/workflows/`.

## Build, Test, and Development Commands

Requires `just`, Rust, and Bun; container builds need Podman or Docker.

- `just check` – full backend and frontend validation.
- `just backend` – format check, clippy with warnings denied, and all-feature tests.
- `just frontend` – frozen-lockfile install, type-check, and production build.
- `just container` – build local image `ghcr.io/niyueee/just-print:local`.
- `just init-hooks` – enable the pre-commit hook running `just check`.
- `cd frontend && bun run dev` – start the Vite dev server.

## Coding Style & Naming Conventions

- **Rust**: rustfmt; clippy `all`/`pedantic` denied, plus `missing_docs`, `unsafe_code`, `unwrap_used`, `expect_used`, `panic`, and `indexing_slicing`. Prefer `Result`/`?` over panics or indexing.
- **Names**: snake_case modules and test functions; public items require doc comments.
- **Frontend**: `tsc -b` type-checking; Preact functional components in PascalCase `.tsx` files, camelCase functions/variables, styles in `app.css`/`index.css`.

## Testing Guidelines

- Backend tests are inline `#[cfg(test)] mod tests` blocks using `#[test]`.
- Name tests descriptively in snake_case, e.g. `merges_roots_and_dedupes_by_device_path`.
- Run them with `cargo test --all-features` or `just backend`.
- No frontend unit tests exist; `just frontend` type-checks and builds, and CI adds container image and API smoke tests.

## Commit & Pull Request Guidelines

- Use Conventional Commits (`feat:`, `fix:`, `docs:`, `ci:`); keep subjects imperative and concise, and explain why in the body.
- Run `just check` before opening a PR; the pre-commit hook enforces it on every commit.
- Summarize the change, link related issues, describe testing, and add screenshots for UI changes. Avoid unrelated edits.

## Security & Configuration

- `JUST_PRINT_TOKEN` is required; the service fails closed when unset or empty.
- Never commit tokens, credentials, or machine-specific paths; environment variables are documented in `README.md`.
