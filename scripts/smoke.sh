#!/bin/sh
# 容器冒烟测试：构建镜像 → 起容器 → 按 api.md 契约跑端到端断言。
#
# 与 CI 对齐：CI 的 smoke 作业直接调用本脚本（podman 或 docker 均可），
# 所以本地跑通过 ≈ CI smoke 通过。用法：
#   scripts/smoke.sh                  # 构建镜像 + 跑全量断言
#   scripts/smoke.sh --no-build       # 跳过构建，复用 SMOKE_IMAGE 指向的镜像
#   SMOKE_IMAGE=foo:bar scripts/smoke.sh --no-build   # 同上
#
# 与 ci.yml 旧版的关系：旧版把断言内联在工作流里，导致本地无法复用；
# 现在两者共用这一份脚本。
set -eu

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

skip_build=0
for arg in "$@"; do
  case "$arg" in
    --no-build) skip_build=1 ;;
    *)
      echo "smoke: unknown argument: $arg" >&2
      exit 2
      ;;
  esac
done

IMAGE="${SMOKE_IMAGE:-just-print:smoke}"
TOKEN="${JUST_PRINT_TOKEN:-ci-smoke-test}"
PORT="${SMOKE_PORT:-18080}"

# 运行时：优先 docker（GitHub runner 原生），其次 podman；都不可用则跳过。
if command -v docker >/dev/null 2>&1; then
  runtime=docker
elif command -v podman >/dev/null 2>&1; then
  runtime=podman
else
  echo "smoke: SKIP (no docker or podman)"
  exit 0
fi

if [ "$skip_build" -eq 0 ]; then
  echo "==> building image with $runtime"
  if [ "$runtime" = docker ]; then
    docker build -f Containerfile -t "$IMAGE" .
  else
    podman build --format docker -f Containerfile -t "$IMAGE" .
  fi
fi

cid=""
cleanup() {
  _status=$?
  if [ -n "$cid" ]; then
    "$runtime" rm -f "$cid" >/dev/null 2>&1 || true
  fi
  # 非零退出时先 dump 容器日志，便于定位是哪一步断言失败。
  # dash 等 POSIX sh 对 `trap ... ERR` 支持不完整（会报 "bad trap"），
  # 所以统一在 EXIT 里按退出状态分流。
  if [ "$_status" -ne 0 ]; then
    echo "=== SMOKE FAILED (exit $_status): container logs (tail 150) ==="
    "$runtime" logs "$cid" 2>&1 | tail -150 || true
  fi
  return 0
}
trap cleanup EXIT

echo "==> starting container"
cid="$("$runtime" run -d --rm -p "127.0.0.1:$PORT:8080" \
  -e "JUST_PRINT_TOKEN=$TOKEN" \
  -e JUST_PRINT_CUPS_PDF=1 \
  "$IMAGE")"

step() { echo "==> smoke: $*"; }

step "wait for /healthz"
for _ in $(seq 1 60); do
  if curl -fsS "http://127.0.0.1:$PORT/healthz" >/dev/null; then
    break
  fi
  sleep 1
done
curl -fsS "http://127.0.0.1:$PORT/healthz" >/dev/null

step "readiness (/readyz queries CUPS)"
for _ in $(seq 1 30); do
  if curl -fsS "http://127.0.0.1:$PORT/readyz" >/dev/null; then
    break
  fi
  sleep 1
done
curl -fsS "http://127.0.0.1:$PORT/readyz" >/dev/null

step "static frontend"
curl -fsS "http://127.0.0.1:$PORT/" | grep -q '<div id="app">'

step "toolchain and CUPS-PDF"
"$runtime" exec "$cid" soffice --version
"$runtime" exec "$cid" gs --version
"$runtime" exec "$cid" lpstat -r | grep -q 'scheduler is running'
"$runtime" exec "$cid" lpstat -p CUPS-PDF | grep -q 'CUPS-PDF'

step "conversions (text + emoji font)"
"$runtime" exec -w /tmp "$cid" sh -c \
  'printf "hello" > sample.txt && soffice --headless --convert-to pdf --outdir /tmp /tmp/sample.txt >/dev/null && test -s /tmp/sample.pdf'
"$runtime" exec -w /tmp "$cid" sh -c \
  'printf "emoji: 😀 🎉 👍 ❤️ ☀️" > /tmp/ci-emoji.txt && soffice --headless --convert-to pdf --outdir /tmp /tmp/ci-emoji.txt >/dev/null && pdffonts /tmp/ci-emoji.pdf | grep -qi symbola'

step "auth: /api without token -> 401"
test "$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$PORT/api/printers")" = 401

step "upload: empty file -> 400"
test "$(curl -s -o /dev/null -w '%{http_code}' \
  -H "Authorization: Bearer $TOKEN" \
  -F 'file=@/dev/null;filename=empty.txt' \
  "http://127.0.0.1:$PORT/api/files")" = 400

step "formats"
formats_json="$(curl -fsS -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:$PORT/api/formats")"
printf '%s' "$formats_json" | grep -q '"extensions"'
printf '%s' "$formats_json" | grep -q '"pdf"'

step "printers"
printers_json="$(curl -s -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:$PORT/api/printers")"
if ! printf '%s' "$printers_json" | grep -q '"id":"CUPS-PDF"'; then
  echo "printers response: $printers_json"
  exit 1
fi

step "upload txt -> preview"
printf 'hello just-print' > /tmp/ci-sample.txt
file_json="$(curl -s -H "Authorization: Bearer $TOKEN" \
  -F 'file=@/tmp/ci-sample.txt;filename=ci-sample.txt' \
  "http://127.0.0.1:$PORT/api/files")"
file_id="$(printf '%s' "$file_json" | sed -n 's/.*"id":"\([0-9a-f]\{32\}\)".*/\1/p')"
if [ -z "$file_id" ]; then
  echo "upload response: $file_json"
  exit 1
fi
curl -fsS -H "Authorization: Bearer $TOKEN" \
  "http://127.0.0.1:$PORT/api/files/$file_id" -o /tmp/ci-preview.pdf
head -c 5 /tmp/ci-preview.pdf | grep -q '%PDF-'

step "markdown -> styled PDF"
# printf 用单引号：反引号与 * 原样写入文件，不参与 shell 展开（SC2016 误报）。
# shellcheck disable=SC2016
printf '# Just Print MD Test\n\n**bold text**\n\n```\ncode block line\n```\n' > /tmp/ci-sample.md
md_json="$(curl -s -H "Authorization: Bearer $TOKEN" \
  -F 'file=@/tmp/ci-sample.md;filename=ci-sample.md' \
  "http://127.0.0.1:$PORT/api/files")"
md_id="$(printf '%s' "$md_json" | sed -n 's/.*"id":"\([0-9a-f]\{32\}\)".*/\1/p')"
if [ -z "$md_id" ]; then
  echo "markdown upload response: $md_json"
  exit 1
fi
md_text="$(curl -fsS -H "Authorization: Bearer $TOKEN" \
  "http://127.0.0.1:$PORT/api/files/$md_id" | "$runtime" exec -i "$cid" pdftotext - -)"
printf '%s' "$md_text" | grep -q 'Just Print MD Test'
printf '%s' "$md_text" | grep -q 'code block line'
if printf '%s' "$md_text" | grep -q '^# Just Print'; then
  echo "raw markdown leaked into PDF"
  exit 1
fi

step "print: missing printer -> 404"
test "$(curl -s -o /dev/null -w '%{http_code}' \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d "{\"file_id\":\"$file_id\",\"printer_id\":\"missing\"}" \
  "http://127.0.0.1:$PORT/api/print")" = 404

step "print over IPP -> wait completed"
print_json="$(curl -s -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' \
  -d "{\"file_id\":\"$file_id\",\"printer_id\":\"CUPS-PDF\",\"options\":{}}" \
  "http://127.0.0.1:$PORT/api/print")"
job_id="$(printf '%s' "$print_json" | sed -n 's/.*"job_id":"\([^"]*\)".*/\1/p')"
if [ -z "$job_id" ]; then
  echo "print response: $print_json"
  exit 1
fi
job_json=""
for _ in $(seq 1 30); do
  job_json="$(curl -s -H "Authorization: Bearer $TOKEN" \
    "http://127.0.0.1:$PORT/api/jobs/$job_id")"
  if printf '%s' "$job_json" | grep -q '"status":"completed"'; then
    break
  fi
  sleep 1
done
if ! printf '%s' "$job_json" | grep -q '"status":"completed"'; then
  echo "job response: $job_json"
  exit 1
fi
"$runtime" exec "$cid" sh -c \
  'find /var/spool/cups-pdf -name "*.pdf" -size +0c | grep -q .'

step "invalid controls -> 400"
test "$(curl -s -o /dev/null -w '%{http_code}' \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d "{\"file_id\":\"$file_id\",\"printer_id\":\"CUPS-PDF\",\"options\":{\"DefinitelyNotAnOption\":\"x\"}}" \
  "http://127.0.0.1:$PORT/api/print")" = 400

step "idempotency: same key replays the same job"
key="smoke-idem-1"
body="{\"file_id\":\"$file_id\",\"printer_id\":\"CUPS-PDF\",\"options\":{}}"
first="$(curl -s -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' -H "Idempotency-Key: $key" \
  -d "$body" "http://127.0.0.1:$PORT/api/print")"
second="$(curl -s -D /tmp/idem-headers -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' -H "Idempotency-Key: $key" \
  -d "$body" "http://127.0.0.1:$PORT/api/print")"
first_id="$(printf '%s' "$first" | sed -n 's/.*"job_id":"\([^"]*\)".*/\1/p')"
second_id="$(printf '%s' "$second" | sed -n 's/.*"job_id":"\([^"]*\)".*/\1/p')"
if [ -z "$first_id" ] || [ "$first_id" != "$second_id" ]; then
  echo "first: $first"
  echo "second: $second"
  exit 1
fi
grep -qi 'idempotency-replayed: true' /tmp/idem-headers

step "jobs list + cancel"
curl -fsS -H "Authorization: Bearer $TOKEN" \
  "http://127.0.0.1:$PORT/api/jobs?limit=5" | grep -q '"jobs"'
# 用 printf 拼换行分隔符：`$'\n'` 是 bash 专有语法，dash（/bin/sh）下不展开。
nl="$(printf '\nx')"
nl="${nl%x}"
cancel_response="$(curl -s -w "${nl}%{http_code}" -X DELETE \
  -H "Authorization: Bearer $TOKEN" \
  "http://127.0.0.1:$PORT/api/jobs/$first_id")"
cancel_status="${cancel_response##*"$nl"}"
cancel_body="${cancel_response%"$nl"*}"
case "$cancel_status" in
  204|404|409) ;;
  *) echo "unexpected cancel status: $cancel_status body: $cancel_body"; exit 1 ;;
esac

step "delete uploaded file"
test "$(curl -s -o /dev/null -w '%{http_code}' -X DELETE \
  -H "Authorization: Bearer $TOKEN" \
  "http://127.0.0.1:$PORT/api/files/$md_id")" = 204

echo "smoke: all checks passed"
