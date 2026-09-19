# 部署指南

## 镜像

镜像从 GitHub Container Registry 拉取：

- `ghcr.io/niyueee/just-print:latest`
- 发布 tag（`v*`）后同时提供 `ghcr.io/niyueee/just-print:vX.Y.Z`

## Docker / Podman 直接运行

```bash
export JUST_PRINT_TOKEN="$(openssl rand -hex 32)"

docker run -d --name just-print -p 8080:8080 \
  -e JUST_PRINT_TOKEN="$JUST_PRINT_TOKEN" \
  ghcr.io/niyueee/just-print:latest
```

使用 USB 打印机时，需要将 USB 设备目录映射进容器：

```bash
docker run -d --name just-print -p 8080:8080 \
  --device /dev/bus/usb:/dev/bus/usb \
  --device /dev/usb:/dev/usb \
  -e JUST_PRINT_TOKEN="$JUST_PRINT_TOKEN" \
  ghcr.io/niyueee/just-print:latest
```

## Compose

```bash
export JUST_PRINT_TOKEN="$(openssl rand -hex 32)"
docker compose -f examples/compose.yaml up -d
podman-compose -f examples/compose.yaml up -d
```

## Podman Quadlet

按 [examples/just-print.container](../examples/just-print.container) 顶部注释部署：

```bash
sudo mkdir -p /etc/containers/systemd /etc/just-print
sudo cp examples/just-print.env.example /etc/just-print/just-print.env
sudo chmod 600 /etc/just-print/just-print.env
sudoedit /etc/just-print/just-print.env   # 必须设置 JUST_PRINT_TOKEN
sudo cp examples/just-print.container /etc/containers/systemd/
sudo systemctl daemon-reload
sudo systemctl enable --now just-print.service
```

## 环境变量

只有 `JUST_PRINT_TOKEN` 是必填项（不设置则 fail-closed 拒绝启动）；其余变量都有
默认值，**通常无需显式设置**。例如 `JUST_PRINT_ADDR`、`JUST_PRINT_WEB_DIR`、
`JUST_PRINT_CUPS_URI` 已由镜像设为下表默认值，`JUST_PRINT_AUTO_USB=1`、
`JUST_PRINT_CUPS_PDF=0`、`JUST_PRINT_DISCOVER_IPP=0` 由入口脚本默认——把它们写进
unit / compose 只是冗余。

> 令牌请放进 `EnvironmentFile`（600 权限），不要写 `Environment=JUST_PRINT_TOKEN=...`：
> 后者会出现在 `systemctl show` / `podman inspect` 中，而且 systemd 对 `Environment=`
> 会做 **% 说明符展开**（`%%` 变成 `%`，含 `%` 的令牌会被静默改写）；`EnvironmentFile`
> 按字面读取，不做说明符展开。生成令牌用 `openssl rand -hex 32`。

最小可用配置（Quadlet）示例：

```ini
[Container]
Image=ghcr.io/niyueee/just-print:latest
PublishPort=8080:8080
EnvironmentFile=/etc/just-print/just-print.env   # 只放 JUST_PRINT_TOKEN
Tmpfs=/tmp
```

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `JUST_PRINT_TOKEN` | 无 | 准入令牌，建议 32 字节以上随机值；未配置或为空时服务拒绝启动（fail-closed） |
| `JUST_PRINT_ADDR` | `0.0.0.0:8080` | 后端监听地址；TLS 由云负载均衡 / Ingress / 反向代理终结 |
| `JUST_PRINT_WEB_DIR` | `/usr/share/just-print/web` | 前端静态文件目录（镜像内已内置） |
| `JUST_PRINT_CUPS_URI` | `http://127.0.0.1:631` | CUPS 地址；`https://` 会使用 IPPS |
| `JUST_PRINT_CUPS_PDF` | `0` | 设为 `1` 时入口脚本添加 `CUPS-PDF` 调试打印机 |
| `JUST_PRINT_DISCOVER_IPP` | `0` | 设为 `1` 时启动 mDNS/Avahi 并用 `ippfind` 自动添加 IPP Everywhere 打印机 |
| `JUST_PRINT_AUTO_USB` | `1` | 启动时自动枚举 USB 打印机并创建 CUPS 队列 |
| `JUST_PRINT_USB_PPD` | 空 | 自动添加 USB 打印机时固定的 PPD；留空按型号匹配（镜像内置 HPLIP PCL 驱动） |
| `JUST_PRINT_USB_OPTIONS` | 空 | USB 队列的额外 PPD 选项（逗号或空格分隔的 `key=value`），用于声明硬件事实，如双面器 `Option1=True` |
| `JUST_PRINT_MAX_UPLOAD_BYTES` | `67108864` | 单文件上传上限（字节，默认 64 MiB） |
| `JUST_PRINT_UPLOAD_SLOTS` | `4` | 并发上传上限 |
| `JUST_PRINT_CONVERSION_SLOTS` | `2` | 并发 LibreOffice 转换上限 |
| `JUST_PRINT_CONVERSION_TIMEOUT_SECS` | `120` | 单次转换超时（秒） |
| `JUST_PRINT_TEMP_TTL_SECS` | `1800` | 无引用临时文件保留时长（秒） |
| `JUST_PRINT_CLEANUP_INTERVAL_SECS` | `60` | 后台清理周期（秒） |
| `JUST_PRINT_IPP_TIMEOUT_SECS` | `15` | 单次 IPP 请求超时（秒） |
| `JUST_PRINT_PRINTER_CACHE_SECS` | `10` | 打印机快照缓存时长（秒） |
| `JUST_PRINT_JOB_CACHE_SECS` | `2` | 任务状态缓存时长（秒） |
| `JUST_PRINT_IDEMPOTENCY_TTL_SECS` | `600` | 幂等键保留时长（秒） |
| `JUST_PRINT_REQUEST_TIMEOUT_SECS` | `300` | 单个 HTTP 请求处理超时（秒） |
| `RUST_LOG` | `info` | tracing 日志过滤（如 `just_print=debug`） |

除 `JUST_PRINT_TOKEN` 外，上表所有变量都可以省略；`examples/` 下的 Compose /
Quadlet / env 示例只保留必填项，可选项均以注释列出。

## 打印机配置

CUPS 在容器启动时由入口脚本拉起，打印机可以通过以下任一方式配置：

- 自动发现：使用 `ippfind` 发现局域网内的 IPP Everywhere 打印机，并以
  `lpadmin -m everywhere` 添加（需 `JUST_PRINT_DISCOVER_IPP=1`）。
- 显式配置：挂载自定义 CUPS 配置，或进入容器后用 `lpadmin` 指定打印机 URI 与
  PPD，例如 `lpadmin -p printer -E -v ipp://... -m /path/to/ppd`。
- 调试：`JUST_PRINT_CUPS_PDF=1` 时入口脚本自动添加 `CUPS-PDF` 打印机，输出 PDF
  到容器内 `/var/spool/cups-pdf/<用户名>`，适合没有真实打印机的 WSL 环境。
- USB 自动配置：默认开启（`JUST_PRINT_AUTO_USB=1`）。入口脚本启动时通过
  `lpinfo` 枚举 USB 打印机，用 USB URI 里的厂商/型号拼出 **IEEE 1284 device-id**，
  交给 `lpinfo -m --device-id` 由 CUPS 匹配 PPD（厂商与型号都要匹配，不会串厂商）；
  仅当厂商是 HP 且 device-id 无结果时，才按型号做保守模糊匹配（镜像内置 HPLIP
  PCL 驱动）；仍无结果才回落到通用 PCL，并在日志里提示。已有同名队列会跳过，
  但仍会应用 `JUST_PRINT_USB_OPTIONS`。
- 手动管理：进入容器后用 `lpadmin` / `lp` 自行管理。

### 驱动是怎么选的（以及为什么无法全自动）

CUPS 选择驱动只有两条正路：

1. **IPP Everywhere / driverless**：打印机支持 IPP（含 IPP-over-USB）时用
   `-m everywhere`，完全不需要厂商驱动，能力（含双面）由设备自己上报。
   `JUST_PRINT_DISCOVER_IPP=1` 的局域网发现走的就是这条路。
2. **传统 PPD**：老式 USB 打印机按 IEEE 1284 device-id 匹配 PPD。镜像里没有的
   厂商专有驱动（联想、爱普生、佳能等）**无法凭空生成**，只能退到镜像自带的通用
   PPD（PCL6 / PCL / PostScript），或用 `JUST_PRINT_USB_PPD` 指定厂商 PPD。

所以“自动加载驱动”的边界是：**设备能自我描述（IPP）→ 全自动；只能给 1284
device-id 且驱动不在镜像里 → 只能通用驱动或人工指定 PPD**。

以联想 LJ4000D 为例（官方参数：PCL6 / BR-Script3，标配双面单元，Debian 无对应包）：

```ini
# 通用 PCL6 单色驱动 + 声明双面器已安装
Environment=JUST_PRINT_USB_PPD=lsb/usr/cupsfilters/pxlmono.ppd
Environment=JUST_PRINT_USB_OPTIONS=OptionDuplex=True
```

镜像自带的通用 PPD 可在容器内用 `lpinfo -m` 查看（如
`lsb/usr/cupsfilters/pxlmono.ppd`、`lsb/usr/cupsfilters/pxlcolor.ppd`、
`drv:///sample.drv/generpcl.ppd`）。

### USB 打印机与 PPD 选项

`lpoptions -p <队列> -l` 列出的是 PPD 的全部选项，但 **`GET /api/printers` 返回的是
CUPS 通过 IPP 上报的能力，两者可能不一致**。典型例子是双面打印：

```text
$ lpoptions -p MFG-MODEL -l
Duplex/2-Sided Printing: *None DuplexNoTumble DuplexTumble
Option1/Duplexer: *False True
```

PPD 里双面受一个**可安装选项**（`Option1` / `OptionDuplex`，人类名 “Duplexer”）
约束；如果它默认 `False`，CUPS 生成 `sides-supported` 时会检测到冲突
（`ppdInstallableConflict`），于是只上报 `one-sided`——界面自然只有「单面」。
硬件是否安装双面器无法从 USB 自动探测，因此需要部署者声明：

- 固件/驱动里声明的厂商与型号必须**同时**匹配才会选中该 PPD（CUPS 的
  `lpinfo -m --device-id`），所以联想之类的机型不会被误配成 HP 驱动。
- 镜像内置 **HPLIP PCL 驱动**（`printer-driver-hpcups`），HP 机型可按 1284
  device-id 命中厂商 PPD；HPLIP 的 PPD 不把 Duplex 绑定到可安装选项，通常直接
  就能上报双面。
- 若某机型的 PPD 仍把双面绑定到可安装选项，用 `JUST_PRINT_USB_OPTIONS` 声明：

  ```ini
  Environment=JUST_PRINT_USB_OPTIONS=Option1=True
  ```

  通用 PCL PPD 的选项名通常是 `Option1`，HPLIP PPD 是 `OptionDuplex`；多个选项用
  逗号分隔。可以在容器内 `grep -iE 'Installable|UIConstraints' /etc/cups/ppd/<队列>.ppd`
  确认实际选项名。
- 想完全固定驱动，用 `JUST_PRINT_USB_PPD`（驱动 URI 或 PPD 文件路径）。
- 改完在界面点「刷新」（或请求 `GET /api/printers?refresh=true`）即可看到新能力。

`JUST_PRINT_USB_OPTIONS` 的合法取值**没有固定清单**，它就是 `lpadmin -o key=value`
的透传，因此由该队列 PPD 的选项决定：

- 查可用的 key/value：`lpoptions -p <队列> -l`，输出形如
  `Keyword/显示名: *默认值 取值1 取值2`——`/` 前是 key，`:` 后是该选项的全部合法
  取值（`*` 标记默认值）。也可直接查 PPD：
  `grep -nE '^\*(OpenUI|Default|Option)' /etc/cups/ppd/<队列>.ppd`。
- **可安装选项**（ppdc 的 `Installable` 指令生成）固定是布尔，取值只有
  `False` / `True`：通用 PCL PPD 是 `Option1=False|True`（显示名 Duplexer），
  HPLIP PPD 是 `OptionDuplex=False|True`（显示名 “Duplexer Installed”）。
- 其它 PPD 选项同样可传（如 `InputSlot=Tray2`、`PageSize=A4`），但它们大多是
  作业级默认值；对「解锁硬件能力」起作用的主要是可安装选项。
- 多个选项用逗号分隔：`JUST_PRINT_USB_OPTIONS=OptionDuplex=True,InputSlot=Tray2`。
  取值按 PPD 原样拼写（大小写敏感）。
- 取值写错时：不带 `=` 的条目会被忽略并打 warning；若 `lpadmin` 拒绝该选项，入口
  脚本会退回「不带任何选项」重新建队列，保证打印机不会因为一个拼写错误而消失。

只有 CUPS 中可见的打印机会出现在 `/api/printers`；前端控制项直接来自 IPP 的
`*-supported` / `*-default` 打印机属性，而不是 PPD / `lpoptions` 名称或 PJL 能力查询。

## 可观测性

- `GET /healthz`：进程存活探针，容器 `HEALTHCHECK` 使用。
- `GET /readyz`：就绪探针，会真实查询 CUPS；适合作为 Ingress/K8s readiness。
- `GET /api/metrics`：Prometheus 文本指标（需要 Bearer 令牌，抓取端配置
  `authorization: { credentials: <token> }` 即可）。
- 日志为 `tracing` 结构化输出，每个请求带 `request_id`；打印提交会写审计日志。

## TLS 与访问控制

- 后端只提供 HTTP，证书在云网关（ALB / Ingress / Caddy / nginx 等）处终结，再
  转发到已发布的 8080 端口。
- 除健康检查外，`/api/*` 请求必须携带 `Authorization: Bearer <token>`（常量时间
  比较）；不使用 Cookie 与会话，无 CSRF 问题。
- CUPS 默认只监听容器内部 `127.0.0.1:631`，不对外暴露。

## USB 设备透传

CUPS 的 `usb` 后端基于 libusb，容器需要映射 `/dev/bus/usb`；个别场景还需要
`/dev/usb/lp*` 设备节点。Compose 与 Quadlet 示例中已给出注释模板。容器启动后
新增的 USB 设备可能不会自动出现在映射中，通常需要按宿主机 udev 策略或重启容器。
自动枚举依赖 `/dev/bus/usb` 已映射进容器。

## 临时文件

文档转换使用容器内 `/tmp`。Compose 与 Quadlet 示例均将其挂为 `tmpfs`：容器重启
即清空，避免残留。后端同时会清理无引用且超过 TTL（默认 30 分钟）的临时文件。

## 常用 CUPS 命令

```bash
lpstat -p -l                                   # 查看打印机与状态
lpoptions -p PRINTER -l                        # 查看可用选项及合法值
lp -d PRINTER -o media=A4 -o sides=two-sided-long-edge file.pdf
lpstat -W not-completed -o PRINTER             # 查看未完成任务
cancel JOB_ID                                  # 取消任务
ippfind                                        # 发现 IPP Everywhere 打印机
```
