# Just Print

![CI](https://github.com/NIyueeE/just-print/actions/workflows/ci.yml/badge.svg)

A printer web service container built with Rust + Preact: the image bundles
CUPS, headless LibreOffice and fonts, converts uploaded documents to PDF and
hands them to CUPS for printing. Supports 172 formats including common office
documents, images, web pages and plain text, plus IPP Everywhere, USB and
network printers. Linux only; ships as a single OCI image compatible with
Docker and Podman.

## Features

- Single image delivery: backend, frontend, CUPS, LibreOffice and fonts in one.
- All 172 LibreOffice-readable formats are converted to PDF with online preview.
- Printers, queues and job states are managed by CUPS; USB printers can be
  auto-queued and the frontend shows common controls.
- The backend talks to CUPS over standard IPP/HTTP with no per-request
  subprocesses, and caches printer/job snapshots with a short TTL.
- `POST /api/print` honours the standard `Idempotency-Key` header, so retries do
  not produce duplicate prints and a lost response is reconciled via `job-name`.
- Job list, status and cancel endpoints; a complete upload → preview → print →
  retry loop.
- Markdown prints as rendered documents: headings, code blocks, tables and
  blockquotes are laid out as a document instead of raw source text.
- Bundled Chinese (Noto CJK) and Symbola emoji fonts, so emoji never render as
  tofu boxes.
- The frontend pre-fills print options with common preferences (A4, highest
  resolution, long-edge duplex) and asks for confirmation before printing.
- Bearer single-token access; the service refuses to start without a token
  (fail-closed); security headers and CSP are on by default.
- `/healthz` liveness, `/readyz` readiness and `/api/metrics` Prometheus metrics.
- Gruvbox-themed frontend with upload, preview, controls and job states, plus
  dark/light themes and full keyboard operation.

## Quick start

```bash
export JUST_PRINT_TOKEN="$(openssl rand -hex 32)"

docker run -d --name just-print -p 8080:8080 \
  -e JUST_PRINT_TOKEN="$JUST_PRINT_TOKEN" \
  ghcr.io/niyueee/just-print:latest
```

For Podman, replace `docker` with `podman`. More deployment options in
[docs/deployment.md](docs/deployment.md):

- Compose: `docker compose -f examples/compose.yaml up -d`
- Podman Quadlet: [examples/just-print.container](examples/just-print.container)
- Build locally: `just container`

## Supported formats

Uploaded formats are converted to PDF before being submitted to CUPS; CUPS
decides the final printer language based on the driver / PPD. The full
extension list is in [docs/api.md](docs/api.md).

## Documentation

- [Web API v1](docs/api.md)
- [Deployment guide](docs/deployment.md)
- [Architecture](docs/architecture.md)
- [Local development](docs/development.md)

## License

[MIT](LICENSE)
