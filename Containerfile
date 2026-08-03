# Containerfile — 同时兼容 docker build 与 podman build。
#
#   docker build -f Containerfile -t ghcr.io/niyueee/just-print:local .
#   podman build -f Containerfile -t ghcr.io/niyueee/just-print:local .

# ---------- 前端构建 ----------
FROM oven/bun:1 AS frontend-builder
WORKDIR /app/frontend
COPY frontend/package.json frontend/bun.lock ./
RUN bun install --frozen-lockfile
COPY frontend/ ./
RUN bun run build

# ---------- 后端构建 ----------
FROM rust:1.96-bookworm AS backend-builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src/ ./src/
RUN cargo build --release --locked

# ---------- 运行镜像 ----------
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        fonts-liberation \
        fonts-noto-cjk \
        libreoffice-writer \
        libreoffice-calc \
        libreoffice-impress \
    && rm -rf /var/lib/apt/lists/*

# 后端负责提供 API 与静态前端；前端产物直接放在镜像内，不再嵌入二进制。
COPY --from=backend-builder /app/target/release/just-print /usr/local/bin/just-print
COPY --from=frontend-builder /app/frontend/dist /usr/share/just-print/web

ENV JUST_PRINT_ADDR=0.0.0.0:8080
ENV JUST_PRINT_WEB_DIR=/usr/share/just-print/web
EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8080/healthz >/dev/null || exit 1

ENTRYPOINT ["/usr/local/bin/just-print"]
