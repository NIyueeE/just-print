# Containerfile — 同时兼容 docker build 与 podman build。
#
#   docker build -f Containerfile -t ghcr.io/niyueee/just-print:local .
#   podman build -f Containerfile -t ghcr.io/niyueee/just-print:local .

# ---------- 前端构建 ----------
FROM docker.io/oven/bun:1 AS frontend-builder
WORKDIR /app/frontend
COPY frontend/package.json frontend/bun.lock ./
RUN bun install --frozen-lockfile
COPY frontend/ ./
RUN bun run build

# ---------- 后端构建 ----------
FROM docker.io/library/rust:1.96-bookworm AS backend-builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src/ ./src/
RUN cargo build --release --locked

# ---------- 运行镜像 ----------
FROM docker.io/library/debian:trixie-slim
# printer-driver-hpcups：HPLIP 的 PCL 驱动与 PPD（含 HP LaserJet 4000 等常见机型），
# 让 USB 队列的型号匹配能命中厂商 PPD 而不是通用 PCL 兜底。
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        avahi-daemon \
        ca-certificates \
        cups \
        cups-client \
        cups-filters \
        curl \
        dbus \
        fonts-liberation \
        fonts-noto-cjk \
        fonts-symbola \
        ghostscript \
        libreoffice-writer \
        libreoffice-calc \
        libreoffice-impress \
        poppler-utils \
        printer-driver-cups-pdf \
        printer-driver-hpcups \
    && rm -rf /var/lib/apt/lists/*

# 后端提供 API 与静态前端；入口脚本负责拉起 CUPS 并可选配置调试/发现打印机。
COPY --chmod=0755 container/entrypoint.sh /usr/local/bin/just-print-entrypoint
COPY --from=backend-builder /app/target/release/just-print /usr/local/bin/just-print
COPY --from=frontend-builder /app/frontend/dist /usr/share/just-print/web

ENV JUST_PRINT_ADDR=0.0.0.0:8080
ENV JUST_PRINT_WEB_DIR=/usr/share/just-print/web
ENV JUST_PRINT_CUPS_URI=http://127.0.0.1:631
EXPOSE 8080

# 就绪探针 /readyz 会真实查询 CUPS；CUPS 不可用时容器标记为 unhealthy。
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
    CMD sh -c 'curl -fsS http://127.0.0.1:8080/readyz >/dev/null'

ENTRYPOINT ["/usr/local/bin/just-print-entrypoint"]
