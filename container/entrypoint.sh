#!/bin/sh
set -eu

# 准备 CUPS 运行目录（容器以 root 启动，cupsd 运行后降权到 lp）。
mkdir -p /var/run/cups /var/spool/cups /var/cache/cups /var/log/cups /run/dbus
chown -R lp:lp /var/run/cups /var/spool/cups /var/cache/cups /var/log/cups 2>/dev/null || true

# cups-pdf 输出目录：后端以 lp 用户创建 /var/spool/cups-pdf/<提交用户>。
mkdir -p /var/spool/cups-pdf
chown lp:lp /var/spool/cups-pdf 2>/dev/null || true

# 可选：局域网 IPP Everywhere 自动发现依赖 mDNS / Avahi。
if [ "${JUST_PRINT_DISCOVER_IPP:-0}" = "1" ]; then
  dbus-daemon --system --fork
  avahi-daemon --daemonize --no-chroot
fi

# 启动 CUPS 并等待调度器就绪。
cupsd
for _ in $(seq 1 30); do
  if [ -S /var/run/cups/cups.sock ]; then
    break
  fi
  sleep 1
done
if ! lpstat -r 2>/dev/null | grep -q "scheduler is running"; then
  echo "error: CUPS scheduler did not start" >&2
  exit 1
fi

# 调试打印机：cups-pdf（无真实打印机环境下的端到端验证）。
if [ "${JUST_PRINT_CUPS_PDF:-0}" = "1" ]; then
  # 固定输出目录，便于在容器内直接检查生成的 PDF。
  sed -i 's|^Out .*|Out /var/spool/cups-pdf/${USER}|' /etc/cups/cups-pdf.conf
  if ! lpstat -p CUPS-PDF >/dev/null 2>&1; then
    lpadmin -p CUPS-PDF -E -v cups-pdf:/ -m lsb/usr/cups-pdf/CUPS-PDF_opt.ppd
  fi
  echo "CUPS-PDF debug printer ready"
fi

# 可选：用 ippfind 发现局域网内的 IPP Everywhere 打印机并自动添加。
if [ "${JUST_PRINT_DISCOVER_IPP:-0}" = "1" ]; then
  timeout 20 ippfind \
    --exec lpadmin -p '{service_name}' -E -v '{}' -m everywhere \; \
    2>/dev/null || true
fi

# 把 JUST_PRINT_USB_OPTIONS（逗号或空格分隔的 key=value）拆成空白分隔列表。
usb_extra_options() {
  printf '%s' "${JUST_PRINT_USB_OPTIONS:-}" | tr ',' ' '
}

# USB URI 的厂商/型号字段可能带 %20 等转义；还原常见的空格与加号。
usb_decode() {
  sed -e 's/+/ /g' -e 's/%20/ /g'
}

# 取 usb://MFG/MDL?serial=... 的厂商字段（第一段路径）。
usb_mfg() {
  printf '%s' "$1" | sed -E 's#^usb://##; s#[?].*$##; s#/.*$##' | usb_decode
}

# 取 usb://MFG/MDL?serial=... 的型号字段（第二段路径）。
usb_mdl() {
  printf '%s' "$1" | sed -E 's#^usb://##; s#[?].*$##; s#^[^/]*/##; s#/.*$##' | usb_decode
}

# 组装 IEEE 1284 device-id 片段，供 `lpinfo -m --device-id` 使用。
usb_device_id() {
  _id=""
  if [ -n "$1" ]; then
    _id="MFG:$1;"
  fi
  if [ -n "$2" ]; then
    _id="${_id}MDL:$2;"
  fi
  printf '%s' "$_id"
}

# 厂商是否为 HP；模糊匹配只在 HP 上使用，避免把其它厂商的型号猜成 HP PPD。
is_hp_vendor() {
  case "$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]')" in
    hp | hewlett-packard | hewlett*) return 0 ;;
    *) return 1 ;;
  esac
}

# 判断 `lpinfo -m` 的某一行是否真的属于该厂商/型号。
# `lpinfo -m --device-id` 在部分 CUPS 版本上可能忽略过滤条件而返回完整列表，
# 直接取第一行会选到无关驱动（例如把 Lenovo 选成 HPLIP 的 "Apollo 2100"），
# 因此这里要求：厂商在 MakeModel 中作为独立词出现，且型号里的首段数字也出现。
ppd_line_matches_device() {
  _line="$1"
  _vendor="$(printf '%s' "$2" | tr '[:upper:]' '[:lower:]' | tr -cd 'a-z0-9')"
  _digits="$(printf '%s' "$3" | grep -o '[0-9][0-9]*' | head -n 1 || true)"
  _mm="$(printf '%s' "$_line" | sed 's/^[^ ]* //' | tr '[:upper:]' '[:lower:]')"
  _mm_words=" $(printf '%s' "$_mm" | tr -c 'a-z0-9' ' ' | tr -s ' ') "
  [ -n "$_vendor" ] || return 1
  case "$_mm_words" in
    *" $_vendor "*) ;;
    *) return 1 ;;
  esac
  if [ -n "$_digits" ]; then
    case "$(printf '%s' "$_mm" | tr -cd '0-9 ')" in
      *"$_digits"*) return 0 ;;
      *) return 1 ;;
    esac
  fi
  return 0
}

# 依据型号从 `lpinfo -m` 中做保守的模糊匹配（仅 HP）：
# 依次尝试完整型号 → 前导 lj 还原为 laserjet → 去掉能力后缀的系列名，
# 并优先厂商 PPD（避免落到 sample.drv 的通用驱动）。
match_ppd() {
  _model="$1"
  _list="$2"
  _base="$_model"
  case "$_base" in
    lj*) _base="laserjet${_base#lj}" ;;
  esac
  _series="$(printf '%s' "$_base" | sed -E 's/([0-9]+)[a-z]+$/\1/')"
  for _candidate in "$_model" "$_base" "$_series"; do
    [ -n "$_candidate" ] || continue
    # 纯数字的系列名过于宽泛，容易误配，直接跳过。
    case "$_candidate" in
      *[a-z]*) ;;
      *) continue ;;
    esac
    _pattern="$(printf '%s' "$_candidate" \
      | sed -E -e 's/[^a-z0-9]+/@/g' -e 's/([a-z])([0-9])/\1@\2/g' -e 's/([0-9])([a-z])/\1@\2/g' \
      | sed 's/@/[-_ ]?/g')"
    _match="$(printf '%s\n' "$_list" | grep -iE "$_pattern" | grep -v 'sample\.drv' | head -n 1 || true)"
    if [ -z "$_match" ]; then
      _match="$(printf '%s\n' "$_list" | grep -iE "$_pattern" | head -n 1 || true)"
    fi
    if [ -n "$_match" ]; then
      printf '%s' "${_match%% *}"
      return 0
    fi
  done
  return 1
}

# 自动添加 USB 打印机队列（默认开启；JUST_PRINT_AUTO_USB=0 关闭）。
# 需要容器映射 /dev/bus/usb 供 lpinfo 枚举。
#
# 驱动选择顺序（与 CUPS 的标准流程一致）：
#   1) 用 USB URI 里的厂商/型号拼出 1284 device-id，交给 `lpinfo -m --device-id`
#      由 CUPS 匹配带 1284DeviceID 的 PPD（厂商+型号都要匹配，不会串厂商）；
#   2) 仍无结果且厂商是 HP 时，才用型号做保守模糊匹配（覆盖设备上报 MFG:HP、
#      而 PPD 写 MFG:Hewlett-Packard 这类对不上的情况）；
#   3) 最后回落到通用 PCL PPD，并提示可用 JUST_PRINT_USB_PPD 固定驱动。
# `JUST_PRINT_USB_OPTIONS` 用于声明硬件相关的 PPD 选项（如双面器
# OptionDuplex=True / Option1=True）。
auto_add_usb_printers() {
  if [ "${JUST_PRINT_AUTO_USB:-1}" = "0" ]; then
    return 0
  fi

  usb_uris="$(lpinfo -v 2>/dev/null | sed -n 's/^direct \(usb:\/\/.*\)$/\1/p' | sort -u)"
  if [ -z "$usb_uris" ]; then
    return 0
  fi

  ppd_list="$(lpinfo -m 2>/dev/null || true)"
  generic_ppd="drv:///sample.drv/generpcl.ppd"
  extra_options="$(usb_extra_options)"

  printf '%s\n' "$usb_uris" | while IFS= read -r uri; do
    mfg="$(usb_mfg "$uri")"
    mdl="$(usb_mdl "$uri")"
    model="$(printf '%s' "$mdl" |
      sed -E 's#[^A-Za-z0-9]+#-#g; s#^-+##; s#-+$##' |
      tr '[:upper:]' '[:lower:]')"
    serial="$(printf '%s' "$uri" |
      sed -n 's/.*[?&]serial=\([^&]*\).*/\1/p' |
      tr '[:upper:]' '[:lower:]' |
      sed 's/[^a-z0-9]//g')"

    if [ -n "$model" ]; then
      name="usb-${model}"
      if [ -n "$serial" ]; then
        name="${name}-${serial}"
      fi
    else
      name="usb-printer"
    fi

    if lpstat -p "$name" >/dev/null 2>&1; then
      # 队列已存在（例如持久化了 /etc/cups）：仍然应用/更新选项声明。
      if [ -n "$extra_options" ]; then
        set -- -p "$name"
        for opt in $extra_options; do
          case "$opt" in
            *=*) set -- "$@" -o "$opt" ;;
            *) echo "warning: 忽略非法的 JUST_PRINT_USB_OPTIONS 项: $opt" >&2 ;;
          esac
        done
        if [ "$#" -gt 2 ]; then
          if lpadmin "$@" >/dev/null 2>&1; then
            echo "updated USB printer options: $name"
          else
            echo "warning: failed to update options for USB printer $name" >&2
          fi
        fi
      fi
      echo "skip USB printer auto-add: $name 已存在"
      continue
    fi

    ppd="${JUST_PRINT_USB_PPD:-$generic_ppd}"
    if [ -z "${JUST_PRINT_USB_PPD:-}" ]; then
      matched=""
      device_id="$(usb_device_id "$mfg" "$mdl")"
      if [ -n "$device_id" ]; then
        candidates_file="$(mktemp)"
        if lpinfo -m --device-id "$device_id" >"$candidates_file" 2>/dev/null; then
          while IFS= read -r line; do
            [ -n "$line" ] || continue
            if ppd_line_matches_device "$line" "$mfg" "$mdl"; then
              matched="$(printf '%s' "$line" | cut -d' ' -f1)"
              break
            fi
          done < "$candidates_file"
        fi
        rm -f "$candidates_file"
      fi
      if [ -z "$matched" ] && is_hp_vendor "$mfg"; then
        matched="$(match_ppd "$model" "$ppd_list" || true)"
      fi
      if [ -n "$matched" ]; then
        ppd="$matched"
      else
        echo "note: 未找到 $mfg $mdl 的专用驱动（device-id: $device_id），使用通用 PPD $generic_ppd" >&2
        echo "note: 如需指定厂商 PPD，设置 JUST_PRINT_USB_PPD（驱动 URI 或 PPD 路径）" >&2
      fi
    fi

    set -- -p "$name" -E -v "$uri" -m "$ppd"
    added_options=0
    for opt in $extra_options; do
      case "$opt" in
        *=*)
          set -- "$@" -o "$opt"
          added_options=1
          ;;
        *) echo "warning: 忽略非法的 JUST_PRINT_USB_OPTIONS 项: $opt" >&2 ;;
      esac
    done
    if lpadmin "$@" >/dev/null 2>&1; then
      echo "auto-added USB printer: $name ($uri, $ppd)"
      continue
    fi
    if [ "$added_options" = "1" ]; then
      # 选项名/值不被该 PPD 接受时，退回不带选项重试，避免因一个拼写错误
      # 导致整台打印机都建不出来。
      echo "warning: lpadmin 拒绝 JUST_PRINT_USB_OPTIONS，改为不带选项重试 $name" >&2
      set -- -p "$name" -E -v "$uri" -m "$ppd"
      if lpadmin "$@" >/dev/null 2>&1; then
        echo "auto-added USB printer (without options): $name ($uri, $ppd)"
        continue
      fi
    fi
    if [ "$ppd" != "$generic_ppd" ]; then
      # PPD 不可用（例如 JUST_PRINT_USB_PPD 写错）时回落到通用 PPD。
      echo "warning: PPD $ppd 不可用，回落到通用 PPD $generic_ppd" >&2
      set -- -p "$name" -E -v "$uri" -m "$generic_ppd"
      if lpadmin "$@" >/dev/null 2>&1; then
        echo "auto-added USB printer (generic PPD): $name ($uri, $generic_ppd)"
        continue
      fi
    fi
    echo "warning: failed to auto-add USB printer $name ($uri, $ppd)" >&2
  done
}

auto_add_usb_printers

exec /usr/local/bin/just-print
