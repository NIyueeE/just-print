#!/bin/sh
# 入口脚本纯逻辑的轻量测试：不启动 CUPS，只校验 PPD 匹配与选项解析。
# 由 `just shell` / CI 调用。
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
sed -n '/^usb_extra_options()/,/^}/p; /^match_ppd()/,/^}/p' "$entrypoint" > "$funcs"
if ! grep -q '^match_ppd()' "$funcs"; then
  echo "error: failed to extract helper functions from entrypoint.sh" >&2
  exit 1
fi
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

PPD_LIST='drv:///sample.drv/generpcl.ppd Generic PCL Laser Printer
drv:///sample.drv/laserjet.ppd HP LaserJet Series PCL 4/5
drv:///hpcups.drv/hp-laserjet_4000_series-pcl3.ppd HP LaserJet 4000 Series pcl3, hpcups 3.22.10
drv:///hpcups.drv/hp-laserjet_4100_series-pcl3.ppd HP LaserJet 4100 Series pcl3, hpcups 3.22.10
drv:///hpcups.drv/hp-officejet_4000_k210.ppd HP Officejet 4000 k210, hpcups 3.22.10
drv:///hpcups.drv/hp-laserjet_p4014dn.ppd HP LaserJet p4014dn, hpcups 3.22.10'

# LJ4000D 的 USB 型号串，应命中 HPLIP 的 4000 系列而不是 Officejet 4000
expect "匹配 lj4000d" "$(match_ppd lj4000d "$PPD_LIST")" \
  "drv:///hpcups.drv/hp-laserjet_4000_series-pcl3.ppd"
# 带厂商标识的完整型号
expect "匹配 hp-laserjet-4000" "$(match_ppd hp-laserjet-4000 "$PPD_LIST")" \
  "drv:///hpcups.drv/hp-laserjet_4000_series-pcl3.ppd"
# 精确型号优先
expect "匹配 p4014dn" "$(match_ppd p4014dn "$PPD_LIST")" \
  "drv:///hpcups.drv/hp-laserjet_p4014dn.ppd"
# 未收录型号返回空串，由调用方回落到 default_ppd
expect "未收录型号返回空" "$(match_ppd zzz9999 "$PPD_LIST")" ""
expect "仅通用驱动时返回空" \
  "$(match_ppd lj4000d 'drv:///sample.drv/generpcl.ppd Generic PCL Laser Printer')" ""

# 选项解析：逗号与空格混用、空值
JUST_PRINT_USB_OPTIONS='Option1=True,OptionDuplex=True'
expect "逗号分隔选项" "$(usb_extra_options)" "Option1=True OptionDuplex=True"
JUST_PRINT_USB_OPTIONS='Option1=True OptionDuplex=True'
expect "空格分隔选项" "$(usb_extra_options)" "Option1=True OptionDuplex=True"
JUST_PRINT_USB_OPTIONS=''
expect "空选项" "$(usb_extra_options)" ""

if [ "$failures" -ne 0 ]; then
  echo "entrypoint helpers: $failures test(s) failed" >&2
  exit 1
fi
echo "entrypoint helpers: all tests passed"
