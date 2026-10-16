#!/bin/sh
# 重门（gate-heavy）：本地 pre-push 与 CI 的 check / smoke 作业共用。
#
# = 完整检查链（fmt / clippy / test / 前端构建 / 入口逻辑）
#   + 真实容器冒烟（scripts/smoke.sh，rootless podman 或 docker）
#
# 用法：
#   scripts/gate-heavy.sh               # 完整重门（含冒烟）
#   scripts/gate-heavy.sh --no-smoke    # 只跑 just check（CI 的 check 作业用）
#
# 与 CI 的对齐：CI 的 check 作业调用本脚本 --no-smoke，smoke 作业调用
# scripts/smoke.sh；两者合起来 ≡ 完整重门。本地通过重门 ≡ 本地通过 CI。
set -eu

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

smoke=1
for arg in "$@"; do
  case "$arg" in
    --no-smoke) smoke=0 ;;
    *)
      echo "gate-heavy: unknown argument: $arg" >&2
      exit 2
      ;;
  esac
done

step() { echo "==> $*"; }

step "完整检查链（just check = backend + frontend + shell）"
just check

if [ "$smoke" -eq 1 ]; then
  step "容器冒烟（真实镜像 + CUPS-PDF 端到端）"
  scripts/smoke.sh
fi

echo "gate-heavy: OK"
