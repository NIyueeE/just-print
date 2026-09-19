# Just Print — Web API v1

相关文档：[部署指南](deployment.md) · [架构与实现](architecture.md) ·
[本地开发](development.md)

除 `/healthz`、`/readyz` 与静态前端外，所有接口都在 `/api` 前缀下，并且必须携带
`Authorization: Bearer <token>`（`JUST_PRINT_TOKEN`）。未带令牌或令牌错误返回
`401`；令牌未配置时服务拒绝启动（fail-closed）。`/api` 下不存在的路径返回统一的
`404 not_found` JSON 信封（不会落入前端静态页面）。

所有 `/api` 响应带 `Cache-Control: no-store`；所有响应带
`X-Content-Type-Options: nosniff`、`Referrer-Policy: no-referrer`、
`X-Frame-Options: DENY`、`Content-Security-Policy` 与 `Permissions-Policy`。
每个响应带 `x-request-id`（客户端可传入同名请求头透传）。

## 通用错误格式

所有错误响应均为 JSON：

```json
{
  "error": {
    "code": "not_found",
    "message": "任务不存在或已失效"
  }
}
```

错误码：

| code | HTTP | 含义 |
| --- | --- | --- |
| `unauthorized` | 401 | 缺少或错误的 Bearer 令牌 |
| `bad_request` | 400 | 请求体/参数不合法 |
| `not_found` | 404 | 文件、打印机或任务不存在 |
| `unsupported_media_type` | 415 | 上传格式不在支持列表 |
| `payload_too_large` | 413 | 上传超过大小限制 |
| `conversion_failed` | 422 | LibreOffice 转换失败 |
| `printer_unavailable` | 409 | 打印机停止/拒绝任务 |
| `conflict` | 409 | 资源状态冲突（文件正在打印/预览） |
| `idempotency_conflict` | 409 | 幂等键正在处理或已用于不同请求；附 `Retry-After` |
| `invalid_controls` | 400 | 选项不是打印机选项目录中的合法值 |
| `bad_gateway` | 502 | CUPS 返回无法处理的响应 |
| `service_unavailable` | 503 | CUPS 暂时不可达；附 `Retry-After` |
| `gateway_timeout` | 504 | CUPS 响应超时 |
| `internal` | 500 | 其它内部错误 |

## 健康检查与就绪

- `GET /healthz` — 进程存活探针，始终返回 `200 ok`（不依赖 CUPS）。
- `GET /readyz` — 就绪探针，通过 IPP 查询 CUPS；成功 `200 ready`，失败
  `503 cups unavailable`。
- `GET /api/metrics` — Prometheus 文本格式指标（需要令牌）。

## POST /api/files

`multipart/form-data`，字段名为 `file`，最大 64 MiB（可由
`JUST_PRINT_MAX_UPLOAD_BYTES` 调整）。请求体流式落盘，不会整体读入内存；
并发上传数由 `JUST_PRINT_UPLOAD_SLOTS` 限制。

支持扩展名与镜像内置 LibreOffice 7.4.7 注册的 `IMPORT` 过滤器一致，另加
Markdown 与纯文本。完整列表由 `GET /api/formats` 返回，不再在客户端硬编码。
`.pdf` 校验 `%PDF-` 魔数后原样通过；`.md` 先渲染为带打印样式的 HTML（原始
HTML 一律转义）再转换；其它格式由镜像内置 LibreOffice 转换为 PDF。

成功（`201 Created`）：

```json
{
  "id": "9f1c0b7a4e2d4f0f9c3b6a1d2e4f5a6b",
  "name": "report.docx",
  "size": 12345
}
```

失败：`400`（缺少文件字段或文件为空）、`413`（超过大小限制）、
`415`（格式不支持）、`422`（转换失败）、`503`（上传/转换通道繁忙）。

## GET /api/files/{id}

流式返回转换后的 PDF（带 `Content-Length`），`Content-Type: application/pdf`，
`Content-Disposition: inline; filename="preview.pdf"`。文件不存在或已过 TTL
返回 `404`。

## DELETE /api/files/{id}

删除尚未被打印/预览引用的上传文件，成功 `204`；文件不存在 `404`；文件正在被
引用时 `409 conflict`。

## GET /api/formats

成功（`200`）：

```json
{
  "extensions": ["pdf", "docx", "md", "..."],
  "max_upload_bytes": 67108864
}
```

## GET /api/printers

可选查询参数 `refresh=true` 跳过服务端缓存（默认缓存 10 秒，避免前端轮询放大
为 CUPS 请求风暴）。

成功（`200`）：

```json
{
  "printers": [
    {
      "id": "CUPS-PDF",
      "name": "CUPS-PDF Printer",
      "state": "idle",
      "accepting_jobs": true,
      "make_and_model": "CUPS-PDF",
      "location": null,
      "options_error": null,
      "options": {
        "media": {
          "kind": "keyword",
          "default": "iso_a4_210x297mm",
          "values": ["iso_a4_210x297mm", "na_letter_8.5x11in"]
        },
        "sides": {
          "kind": "keyword",
          "default": "one-sided",
          "values": ["one-sided", "two-sided-long-edge", "two-sided-short-edge"]
        },
        "print-quality": {
          "kind": "enum",
          "default": "normal",
          "values": [
            { "value": 3, "name": "draft" },
            { "value": 4, "name": "normal" },
            { "value": 5, "name": "high" }
          ]
        },
        "printer-resolution": {
          "kind": "resolution",
          "default": "600x600dpi",
          "values": [
            { "cross_feed": 300, "feed": 300, "units": 3, "label": "300x300dpi" },
            { "cross_feed": 600, "feed": 600, "units": 3, "label": "600x600dpi" }
          ]
        },
        "copies": { "kind": "integer", "default": "1", "min": 1, "max": 999 },
        "number-up": { "kind": "integer_choices", "default": "1", "values": [1, 2, 4, 6, 9, 16] }
      }
    }
  ]
}
```

- `id` 是 CUPS 打印机名，也是提交任务时的 `printer_id`。
- `state`：`idle` / `printing` / `stopped` / `disabled`（来自 IPP
  `printer-state` 与 `printer-is-accepting-jobs`）。
- `options` 的键使用标准 IPP `job-*` 属性名；`kind` 决定前端控件类型与提交时
  的 IPP 值编码。`options_error` 非空表示该打印机的选项目录读取失败（降级为空）。
- 打印机没有可用选项时 `options` 为空对象。
- CUPS 中没有任何打印机时返回 `200 {"printers":[]}`，不是错误。

## POST /api/print

请求体：

```json
{
  "file_id": "9f1c0b7a4e2d4f0f9c3b6a1d2e4f5a6b",
  "printer_id": "CUPS-PDF",
  "options": {
    "media": "iso_a4_210x297mm",
    "sides": "two-sided-long-edge",
    "copies": "2"
  }
}
```

`options` 可省略；提供时每个键必须来自 `/api/printers` 返回的选项目录，值必须
是该选项的合法取值，否则 `400 invalid_controls`。

### 幂等性

打印是物理副作用，因此**强烈建议**携带标准 `Idempotency-Key` 请求头（建议使用
UUID）：

- 同一键 + 相同负载在保留期（默认 10 分钟，`JUST_PRINT_IDEMPOTENCY_TTL_SECS`）
  内只产生一个 CUPS 任务；后续请求回放首次响应，并带
  `Idempotency-Replayed: true`。
- 同一键 + 不同负载返回 `409 idempotency_conflict`。
- 同一键的首次请求仍在处理中时返回 `409 idempotency_conflict` 与
  `Retry-After: 1`。
- 提交超时或连接失败时，服务端会用写入 `job-name` 的唯一标记在 CUPS 中找回
  已创建的任务，避免重复出纸。
- 不携带该头时每次请求都会创建一个新任务。

成功（`202 Accepted`）：

```json
{
  "job_id": "CUPS-PDF-8",
  "printer_id": "CUPS-PDF",
  "status": "queued",
  "created_at_ms": 1783173600000,
  "file_name": "report.pdf"
}
```

`job_id` 是应用层任务 id，形式为 `<printer>-<CUPS 任务号>`。

失败：`404 not_found`（文件或打印机不存在）、`400 invalid_controls`、
`409 printer_unavailable` / `409 idempotency_conflict`、`503`（CUPS 不可用）、
`504`（CUPS 超时）。

## GET /api/jobs

查询参数：`limit`（1–200，默认 50）、`active`（`true` 时只返回排队/打印中）。

成功（`200`）：

```json
{
  "jobs": [
    {
      "id": "CUPS-PDF-8",
      "printer_id": "CUPS-PDF",
      "name": "[1a2b3c4d] report.pdf",
      "file_id": "9f1c0b7a4e2d4f0f9c3b6a1d2e4f5a6b",
      "file_name": "report.pdf",
      "status": "printing",
      "error": null,
      "created_at_ms": 1783173600000,
      "options": { "media": "iso_a4_210x297mm" }
    }
  ]
}
```

`status` 为 `queued` / `printing` / `completed` / `failed` / `canceled`；任务已被
CUPS 清理时为 `null`。`file_id` / `file_name` / `options` 来自应用层登记表，
服务重启后可能缺失（CUPS 中的任务仍在）。

## GET /api/jobs/{id}

`{id}` 使用 `<printer>-<job>` 形式；纯数字任务号仅在登记表中唯一匹配时接受，
否则返回 `400`（多台打印机任务号可能重复）。

成功（`200`）：返回单个 `JobView`（结构同上）。任务在 CUPS spool 中已被清理
返回 `404`。

## DELETE /api/jobs/{id}

取消任务。成功 `204`；任务不存在 `404`；CUPS 拒绝取消返回 `409`/`502`。
