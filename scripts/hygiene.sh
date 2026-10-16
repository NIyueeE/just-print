#!/bin/sh
# 仓库卫生检查：跨编辑器/跨平台提交的常见污染。
#  - 行尾必须是 LF（CRLF 会被 git 的 autocrlf 转换，但直接提交仍会脏）
#  - 不允许尾随空白（对 .sh 尤其致命：`if [ ... ]; then␠`）
#  - 文本文件必须以换行结尾
#  - .sh / .githooks 必须带可执行位（git 100755）
#  - 单文件不超过 1 MiB（大文件应进 release asset，不进 git）
#
# 用 while read 循环 + 临时文件计数而不是管道里的 while（POSIX sh 下管道
# 子 shell 无法回传变量），保证问题能被正确汇总成退出码。
set -eu

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

problems=0
fail_file="$(mktemp)"
trap 'rm -f "$fail_file"' EXIT

# 行尾 / 尾随空白 / 结尾换行：只检查文本文件（二进制用 grep -qI 跳过）。
git ls-files > .hygiene-files.$$
trap 'rm -f .hygiene-files.$$ "$fail_file"' EXIT
while IFS= read -r f; do
  [ -f "$f" ] || continue
  if ! grep -qI '' "$f" 2>/dev/null; then
    continue
  fi
  if grep -q "$(printf '\r')" "$f" 2>/dev/null; then
    echo "hygiene: CRLF line ending in $f" >&2
    problems=$((problems + 1))
  fi
  if grep -n '[[:blank:]]$' "$f" >/dev/null 2>&1; then
    echo "hygiene: trailing whitespace in $f" >&2
    problems=$((problems + 1))
  fi
  if [ -s "$f" ] && [ "$(tail -c1 "$f" | od -An -c | tr -d ' ')" != '\n' ]; then
    echo "hygiene: missing final newline in $f" >&2
    problems=$((problems + 1))
  fi
done < .hygiene-files.$$

# 可执行位：所有 .sh 与 .githooks 里的钩子必须 100755。
git ls-files '*.sh' '.githooks/*' > .hygiene-scripts.$$
while IFS= read -r f; do
  [ -f "$f" ] || continue
  mode="$(git ls-files -s -- "$f" | awk '{print $1}')"
  if [ "$mode" != "100755" ]; then
    echo "hygiene: $f should be executable (mode $mode, want 100755): run 'chmod +x $f'" >&2
    problems=$((problems + 1))
  fi
done < .hygiene-scripts.$$

# 体积：>1 MiB 的单文件。
while IFS= read -r f; do
  [ -f "$f" ] || continue
  size="$(wc -c < "$f" | tr -d ' ')"
  if [ "$size" -gt 1048576 ]; then
    echo "hygiene: $f is $size bytes (>1 MiB); keep large artifacts out of git" >&2
    problems=$((problems + 1))
  fi
done < .hygiene-files.$$

rm -f .hygiene-files.$$ .hygiene-scripts.$$

if [ "$problems" -ne 0 ]; then
  echo "hygiene: FAILED ($problems problem(s))" >&2
  exit 1
fi
echo "hygiene: OK"
