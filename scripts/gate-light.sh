#!/bin/sh
# 轻门（gate-light）：本地 pre-commit 与 CI 的 lint 作业共用。
#
# 设计原则：
#   - 只做「秒级到几十秒级」的检查，不触发 cargo 重编译（fmt --check 会读
#     元数据但不重编译；clippy/test/build 留给重门 gate-heavy）。
#   - CI 设置 PROBE=strict，此时缺失的检查工具视为失败而不是跳过，保证 CI
#     结果可解释；本地默认宽松，缺工具只打 SKIP。
#   - 同时 lint CI 配置自身（actionlint / hadolint / zizmor），让「检查检查者」
#     也在同一道门里跑，防止门禁自己先烂掉。
#   - 任何一步失败立即退出（set -e），并把失败步骤打清楚。
set -eu

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

if [ "${PROBE:-}" = "strict" ]; then
  strict=1
else
  strict=0
fi

# run_tool <工具名> <命令...>：工具缺失时按 strict/skip 处理。
run_tool() {
  _tool="$1"
  shift
  if command -v "$_tool" >/dev/null 2>&1; then
    echo "==> run: $_tool $*"
    "$@"
  elif [ "$strict" -eq 1 ]; then
    echo "error: required gate tool '$_tool' is not installed" >&2
    exit 1
  else
    echo "==> skip: $_tool (not installed; set PROBE=strict or install it)"
  fi
}

step() { echo "==> step: $*"; }

step "仓库卫生（换行符/尾随空白/可执行位/体积）"
scripts/hygiene.sh

step "拼写检查（typos；锁文件是生成物，排除）"
run_tool typos typos --format brief --exclude Cargo.lock --exclude frontend/bun.lock

step "Rust 格式（cargo fmt --check）"
cargo fmt --all -- --check

# 前端工具（prettier / eslint / tsc / vitest）都来自 node_modules/.bin。
# `bun run` 只有在该目录存在时才会把它加进 PATH；本地仓库通常已经装过，
# 而 CI 的 lint 作业不经过 `just check`（那里才跑 bun install），必须在这里
# 补一次。--frozen-lockfile 保证与 lockfile 完全一致，重复执行是幂等的。
step "前端依赖（frozen-lockfile）"
(cd frontend && bun install --frozen-lockfile)

step "前端格式（prettier --check）"
(cd frontend && bun run format:check)

step "前端 lint（eslint）"
(cd frontend && bun run lint)

step "入口脚本（shellcheck + 纯逻辑测试）"
run_tool shellcheck shellcheck container/entrypoint.sh container/entrypoint.test.sh
sh -n container/entrypoint.sh
sh container/entrypoint.test.sh

step "CI 配置 lint（actionlint / hadolint / zizmor）"
run_tool actionlint actionlint
run_tool hadolint hadolint Containerfile
run_tool zizmor zizmor --persona auditor .github/workflows/

step "未跟踪文件（仅供回顾，不算失败）"
git ls-files --others --exclude-standard || true

echo "gate-light: OK"
