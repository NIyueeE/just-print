# Just Print

![CI](https://github.com/NIyueeE/just-print/actions/workflows/ci.yml/badge.svg)

基于 Rust + Preact 构建的打印机 Web 服务容器：镜像内置 CUPS、headless
LibreOffice 与字体，上传的文档统一转换为 PDF 后交给 CUPS 打印。支持常见办公
文档、图片、网页与纯文本等 172 种格式，以及 IPP Everywhere、USB 和网络打印机。
仅支持 Linux，交付形态为单一 OCI 镜像，兼容 Docker 与 Podman。

## 特性

- 单镜像交付：后端、前端、CUPS、LibreOffice 与字体一体。
- 172 种 LibreOffice 可读格式统一转 PDF，并提供在线预览。
- 打印机、队列与任务状态由 CUPS 管理；USB 打印机可自动建队列，前端展示常见控制项。
- 后端通过标准 IPP over HTTP 直接与 CUPS 通信，无逐请求子进程开销；打印机与任务状态带短 TTL 缓存。
- 打印提交支持标准 `Idempotency-Key`：重试不会重复出纸，并可在响应丢失后按 `job-name` 对账找回任务。
- 任务列表、状态查询与取消接口；上传、预览与失败重打的完整闭环。
- Markdown 按渲染样式打印：标题、代码块、表格与引用以文档排版输出，而非源码文本。
- 容器内置中文（Noto CJK）与 Symbola 表情符号字体，emoji 不会变成豆腐块。
- 前端按常用偏好预填打印选项：A4、最高分辨率、双面长边装订；提交前有确认对话框。
- Bearer 单令牌准入；未配置令牌时拒绝启动（fail-closed）；安全响应头与 CSP 默认开启。
- `/healthz` 存活探针、`/readyz` 就绪探针与 `/api/metrics` Prometheus 指标。
- 前端采用 Gruvbox 配色，提供上传、预览、控制项与任务状态展示；支持暗/亮主题与键盘操作。

## 快速开始

```bash
export JUST_PRINT_TOKEN="$(openssl rand -hex 32)"

docker run -d --name just-print -p 8080:8080 \
  -e JUST_PRINT_TOKEN="$JUST_PRINT_TOKEN" \
  ghcr.io/niyueee/just-print:latest
```

Podman 只需将 `docker` 换成 `podman`。更多部署方式见
[docs/deployment.md](docs/deployment.md)：

- Compose：`docker compose -f examples/compose.yaml up -d`
- Podman Quadlet：[examples/just-print.container](examples/just-print.container)
- 本地构建镜像：`just container`

## 支持格式

上传格式统一转换为 PDF 后提交给 CUPS；CUPS 根据驱动 / PPD 决定最终发送给打印机的
语言。完整扩展名列表见 [docs/api.md](docs/api.md)。

## 文档

- [Web API v1](docs/api.md)
- [部署指南](docs/deployment.md)
- [架构与实现](docs/architecture.md)
- [本地开发与检查](docs/development.md)

## License

[MIT](LICENSE)
