[private]
default:
    @just --list

# 后端 + 前端完整检查
check: backend frontend

# 后端完整检查链：格式、静态检查、测试
backend:
    cargo fmt --all -- --check
    cargo clippy --all-targets --all-features -- -D warnings
    cargo test --all-features

# 前端完整检查链：依赖校验、类型检查、生产构建
frontend:
    cd frontend && bun install --frozen-lockfile
    cd frontend && bun run build

# 构建容器镜像（自动选择 podman 或 docker）
container:
    @if command -v podman >/dev/null 2>&1; then podman build --format docker -f Containerfile -t ghcr.io/niyueee/just-print:local .; elif command -v docker >/dev/null 2>&1; then docker build -f Containerfile -t ghcr.io/niyueee/just-print:local .; else echo "error: podman or docker is required" >&2; exit 1; fi

# 构建并以前台运行 cups-pdf 调试容器（需要 JUST_PRINT_TOKEN 环境变量）
debug:
    just container
    @test -n "${JUST_PRINT_TOKEN:-}" || (echo "error: JUST_PRINT_TOKEN is required" >&2; exit 1)
    @podman rm -f just-print-debug >/dev/null 2>&1 || true
    podman run --rm --name just-print-debug -p 8080:8080 \
        -e JUST_PRINT_TOKEN="$JUST_PRINT_TOKEN" \
        -e JUST_PRINT_CUPS_PDF=1 \
        ghcr.io/niyueee/just-print:local

# 启用 pre-commit 钩子（首次克隆后执行一次）
init-hooks:
    git config core.hooksPath .githooks
