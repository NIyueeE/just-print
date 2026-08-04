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

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `JUST_PRINT_TOKEN` | 无 | 准入令牌，建议 32 字节以上随机值；未配置或为空时服务拒绝启动（fail-closed） |
| `JUST_PRINT_ADDR` | `0.0.0.0:8080` | 后端监听地址；容器内直接对外监听，TLS 由云负载均衡 / Ingress / 反向代理终结 |
| `JUST_PRINT_WEB_DIR` | `/usr/share/just-print/web` | 前端静态文件目录（镜像内已内置，一般无需修改） |
| `JUST_PRINT_CUPS_URI` | `http://127.0.0.1:631` | 容器内 CUPS 服务地址；一般无需修改 |
| `JUST_PRINT_CUPS_PDF` | `0` | 设为 `1` 时入口脚本自动添加 `CUPS-PDF` 调试打印机（cups-pdf，无真实打印机环境验证用） |
| `JUST_PRINT_DISCOVER_IPP` | `0` | 设为 `1` 时启动 mDNS/Avahi，并用 `ippfind` 自动添加局域网 IPP Everywhere 打印机 |

## 打印机配置

CUPS 在容器启动时由入口脚本拉起，打印机可以通过以下任一方式配置：

- 自动发现：使用 `ippfind` 发现局域网内的 IPP Everywhere 打印机，并以
  `lpadmin -m everywhere` 添加（需 `JUST_PRINT_DISCOVER_IPP=1`）。
- 显式配置：挂载自定义 CUPS 配置，或进入容器后用 `lpadmin` 指定打印机 URI 与
  PPD，例如 `lpadmin -p printer -E -v ipp://... -m /path/to/ppd`。
- 调试：`JUST_PRINT_CUPS_PDF=1` 时入口脚本自动添加 `CUPS-PDF` 打印机，输出 PDF
  到容器内 `/var/spool/cups-pdf/<用户名>`，适合没有真实打印机的 WSL 环境。
- 手动管理：进入容器后用 `lpadmin` / `lp` 自行管理。

只有 CUPS 中可见的打印机会出现在 `/api/printers`；前端控制项来自 `lpoptions -l`
与 IPP 属性，而不是 PJL 能力查询。

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
