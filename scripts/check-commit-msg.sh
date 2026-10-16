#!/bin/sh
# 提交信息约定：Conventional Commits 子集 + 主题长度提示。
#
# 允许：
#   <type>(<scope>): <subject>          type ∈ feat|fix|docs|refactor|perf|test|
#                                            build|ci|chore|revert
#   Revert "..."                         （git revert 自动生成）
#   Merge ...                            （merge commit 由 git 自己写）
#   fixup!/squash! 前缀                  （交互式 rebase 用）
set -eu

[ $# -ge 1 ] || {
  echo "usage: check-commit-msg.sh <commit-msg-file>" >&2
  exit 2
}
msg_file="$1"

# 取首个非注释、非空行作为主题行。
subject="$(grep -v '^#' "$msg_file" | sed -n '/[^[:space:]]/ {p; q}')"

case "$subject" in
  Revert\ *)
    echo "commit-msg: OK (revert)"
    exit 0
    ;;
  Merge\ *|\"Merge\ *)
    echo "commit-msg: OK (merge)"
    exit 0
    ;;
  fixup!\ *|squash!\ *|amend!\ *)
    echo "commit-msg: OK ($(printf '%s' "$subject" | cut -d! -f1)) "
    exit 0
    ;;
esac

# Conventional Commits：type(scope)!: subject
# 字符类里避免 `a-z0-9` 之外的区间混写（某些 grep 会把 `,\-` 当成区间报
# "Invalid range end"），这里显式枚举。
if printf '%s' "$subject" | grep -Eq '^(feat|fix|docs|refactor|perf|test|build|ci|chore|revert)(\([a-z0-9,/._-]+\))?!?: .+$'; then
  len="$(printf '%s' "$subject" | wc -c | tr -d ' ')"
  if [ "$len" -gt 73 ]; then
    echo "commit-msg: hint: subject is $len chars, keep it <= 72" >&2
  fi
  echo "commit-msg: OK"
  exit 0
fi

echo "commit-msg: rejected subject: $subject" >&2
echo "commit-msg: expected Conventional Commits, e.g. 'fix(backend): keep PDF uploads out of the LibreOffice slot queue'" >&2
exit 1
