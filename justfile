[private]
default:
    @just --list

# 以环境变量/配置文件为变量源的复合配方
set unstable := true
set dotenv-load := true

# ---------------------------------------------------------------- 环境探测
#
# 门禁脚本用 PROBE 标记「工具缺失 vs 检查失败」：可选工具缺失时打印 SKIP，
# 默认（strict，即 CI）时缺失会失败。本地默认宽松（PROBE 未设置）。

_require_vars := `
if [ "${PROBE:-}" = "strict" ]; then
  :
else
  export PROBE=skip
fi
`

# ---------------------------------------------------------------- 分层门禁
#
#   gate-light  轻门：跑得起、不进 cargo 重编译。本地 pre-commit 钩子用它，
#                目标是「快的检查留在本地」，几十秒内给出反馈。
#   gate-heavy  重门：完整 check（fmt/clippy/test/build + smoke 冒烟脚本）。
#                本地 pre-push 钩子与 CI 的 check / smoke 作业都用它。
#
# 两者都不依赖 Docker 守护进程：冒烟脚本用 rootless podman 跑容器，
# docker 环境等价可用（见 scripts/gate-heavy.sh）。

# 轻门（pre-commit）：静态/快速检查，含 CI 配置自身的 lint。
gate-light:
    {{ _require_vars }}
    scripts/gate-light.sh

# 重门（pre-push / CI）：完整后端+前端+入口检查 + 真实容器冒烟。
gate-heavy:
    {{ _require_vars }}
    scripts/gate-heavy.sh

# ---------------------------------------------------------------- 检查链
#
# CI 与本地钩子调用的是这些「可组合」目标，避免工作流里再写一遍命令。

# 后端完整检查链：格式、静态检查、测试
backend:
    cargo fmt --all -- --check
    cargo clippy --all-targets --all-features -- -D warnings
    cargo test --all-features

# 前端完整检查链：依赖校验、类型检查、lint、格式、测试、生产构建
frontend:
    cd frontend && bun install --frozen-lockfile
    cd frontend && bun run typecheck
    cd frontend && bun run lint
    cd frontend && bun run format:check
    cd frontend && bun run test:run
    cd frontend && bun run build

# 入口脚本语法检查 + PPD 匹配/选项解析的纯逻辑测试
shell:
    sh -n container/entrypoint.sh
    sh container/entrypoint.test.sh

# 完整检查链（CI 的 check 作业用它）
check: backend frontend shell

# 提交信息约定校验
commit-msg:
    {{ _require_vars }}
    scripts/check-commit-msg.sh

# ---------------------------------------------------------------- 依赖与镜像
#
# 依赖审计只做 advisory 与 license 门禁（它曾在漏洞被上游修复前持续报红）。
# Rust 的 lockfile/去重（bans）、前端与镜像的漏洞扫描由 CI 对应作业负责。

# 依赖安全审计（需要 cargo-audit 与 bun；仅 advisory + license）
# 注：cargo-audit 新版（0.22+）已把 advisory/license 合并为默认子命令，
# 旧写法 `audit advisories licenses` 会直接报 "unrecognized subcommand"。
audit:
    {{ _require_vars }}
    cargo audit --deny warnings
    cd frontend && bun audit --audit-level=high

# 构建容器镜像（自动选择 podman 或 docker）
container:
    @if command -v podman >/dev/null 2>&1; then podman build --format docker -f Containerfile -t ghcr.io/niyueee/just-print:local .; elif command -v docker >/dev/null 2>&1; then docker build -f Containerfile -t ghcr.io/niyueee/just-print:local .; else echo "error: podman or docker is required" >&2; exit 1; fi

# ---------------------------------------------------------------- 钩子管理
#
# hooksPath 用 git config --local 写入（本仓库独有，--global 是给「所有仓库都
# 用同一套钩子」用的）。执行过一次后，提交前跑轻门、推送前跑重门。
# CI 不依赖这些钩子，本地与 CI 的对齐由 gate-light/gate-heavy 保证。

# 启用本地 git 钩子（首次克隆后执行一次）
init-hooks:
    git config --local core.hooksPath .githooks
    @echo "hooks enabled (core.hooksPath=.githooks): pre-commit -> just gate-light, pre-push -> just gate-heavy"

# 卸载本地 git 钩子
remove-hooks:
    git config --local --unset core.hooksPath 2>/dev/null || true
    @echo "hooks disabled"
