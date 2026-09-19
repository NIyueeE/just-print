#!/bin/sh
# 入口脚本纯逻辑的轻量测试：不启动 CUPS，只校验 USB URI 解析、device-id 拼装、
# 厂商判定与 PPD 模糊匹配。由 `just shell` / CI 调用。
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
  -e '/^is_hp_vendor()/,/^}/p' \
  -e '/^match_ppd()/,/^}/p' \
  "$entrypoint" > "$funcs"
for required in usb_device_id usb_mfg usb_mdl is_hp_vendor match_ppd; do
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

# 厂商判定：模糊匹配只允许 HP，联想必须走 device-id/通用驱动
expect_true is_hp_vendor HP
expect_true is_hp_vendor hewlett-packard
expect_false is_hp_vendor Lenovo
expect_false is_hp_vendor Brother

# 模糊匹配（仅 HP 路径）仍按型号+系列命中厂商 PPD，且不误配同数字的其它系列
PPD_LIST='drv:///sample.drv/generpcl.ppd Generic PCL Laser Printer
drv:///sample.drv/laserjet.ppd HP LaserJet Series PCL 4/5
drv:///hpcups.drv/hp-laserjet_4000_series-pcl3.ppd HP LaserJet 4000 Series pcl3, hpcups 3.22.10
drv:///hpcups.drv/hp-officejet_4000_k210.ppd HP Officejet 4000 k210, hpcups 3.22.10
drv:///hpcups.drv/hp-laserjet_p4014dn.ppd HP LaserJet p4014dn, hpcups 3.22.10'
expect "匹配 lj4000d" "$(match_ppd lj4000d "$PPD_LIST")" \
  "drv:///hpcups.drv/hp-laserjet_4000_series-pcl3.ppd"
expect "匹配 p4014dn" "$(match_ppd p4014dn "$PPD_LIST")" \
  "drv:///hpcups.drv/hp-laserjet_p4014dn.ppd"
expect "未收录型号返回空" "$(match_ppd zzz9999 "$PPD_LIST")" ""
expect "仅通用驱动时返回空" \
  "$(match_ppd lj4000d 'drv:///sample.drv/generpcl.ppd Generic PCL Laser Printer')" ""

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
