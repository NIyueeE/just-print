#!/bin/sh
# 入口脚本纯逻辑的轻量测试：不启动 CUPS，只校验 USB URI 解析、device-id 拼装与
# 候选 PPD 校验。由 `just shell` / CI 调用。
set -eu

script_dir="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
entrypoint="$script_dir/entrypoint.sh"

if [ ! -r "$entrypoint" ]; then
  echo "error: cannot read $entrypoint" >&2
  exit 1
fi

# 只抽取纯函数定义，避免执行 cupsd / lpadmin 等启动逻辑。
funcs="$(mktemp)"
trap 'rm -f "$funcs"' EXIT
sed -n \
  -e '/^usb_extra_options()/,/^}/p' \
  -e '/^usb_decode()/,/^}/p' \
  -e '/^usb_mfg()/,/^}/p' \
  -e '/^usb_mdl()/,/^}/p' \
  -e '/^usb_device_id()/,/^}/p' \
  -e '/^ppd_line_matches_device()/,/^}/p' \
  "$entrypoint" > "$funcs"
for required in usb_device_id usb_mfg usb_mdl ppd_line_matches_device; do
  if ! grep -q "^$required()" "$funcs"; then
    echo "error: failed to extract $required() from entrypoint.sh" >&2
    exit 1
  fi
done
# shellcheck disable=SC1090
. "$funcs"

failures=0
expect() {
  if [ "$2" != "$3" ]; then
    echo "FAIL: $1 (expected '$3', got '$2')" >&2
    failures=$((failures + 1))
  else
    echo "ok: $1"
  fi
}
expect_true() {
  if "$@" >/dev/null 2>&1; then
    echo "ok: $* (expected true)"
  else
    echo "FAIL: $* (expected true)" >&2
    failures=$((failures + 1))
  fi
}
expect_false() {
  if "$@" >/dev/null 2>&1; then
    echo "FAIL: $* (expected false)" >&2
    failures=$((failures + 1))
  else
    echo "ok: $* (expected false)"
  fi
}

# USB URI 解析：联想实机（无转义）与 HP（%20）两种形态
lenovo_uri='usb://Lenovo/LJ4000D?serial=00000lp05609863'
expect "解析联想厂商" "$(usb_mfg "$lenovo_uri")" "Lenovo"
expect "解析联想型号" "$(usb_mdl "$lenovo_uri")" "LJ4000D"
hp_uri='usb://HP/LaserJet%204000?serial=ABC123'
expect "解析 HP 厂商" "$(usb_mfg "$hp_uri")" "HP"
expect "解析 HP 型号（%20 还原）" "$(usb_mdl "$hp_uri")" "LaserJet 4000"

# 1284 device-id 拼装
expect "device-id（厂商+型号）" "$(usb_device_id Lenovo LJ4000D)" "MFG:Lenovo;MDL:LJ4000D;"
expect "device-id（仅型号）" "$(usb_device_id "" LJ4000D)" "MDL:LJ4000D;"
expect "device-id（都为空）" "$(usb_device_id "" "")" ""

# 候选校验：拒绝与厂商/型号不符的 PPD（联想实机上曾误选 HPLIP 的 Apollo 2100）
APOLLO='drv:///hpcups.drv/apollo-2100.ppd Apollo 2100, hpcups 3.22.10'
HP4000='drv:///hpcups.drv/hp-laserjet_4000_series-pcl3.ppd HP LaserJet 4000 Series pcl3, hpcups 3.22.10'
HP4014='drv:///hpcups.drv/hp-laserjet_p4014dn.ppd HP LaserJet p4014dn, hpcups 3.22.10'
expect_false ppd_line_matches_device "$APOLLO" Lenovo LJ4000D
expect_false ppd_line_matches_device "$HP4000" Lenovo LJ4000D
expect_true ppd_line_matches_device "$HP4000" HP "LaserJet 4000"
expect_true ppd_line_matches_device "$HP4014" HP p4014dn
expect_false ppd_line_matches_device "$HP4014" HP "LaserJet 4000"
expect_true ppd_line_matches_device "$APOLLO" Apollo "Apollo 2100"

# 选项解析：逗号与空格混用、空值
JUST_PRINT_USB_OPTIONS='OptionDuplex=True,Option1=True'
expect "逗号分隔选项" "$(usb_extra_options)" "OptionDuplex=True Option1=True"
JUST_PRINT_USB_OPTIONS='OptionDuplex=True Option1=True'
expect "空格分隔选项" "$(usb_extra_options)" "OptionDuplex=True Option1=True"
JUST_PRINT_USB_OPTIONS=''
expect "空选项" "$(usb_extra_options)" ""

if [ "$failures" -ne 0 ]; then
  echo "entrypoint helpers: $failures test(s) failed" >&2
  exit 1
fi
echo "entrypoint helpers: all tests passed"
