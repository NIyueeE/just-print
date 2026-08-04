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

exec /usr/local/bin/just-print
