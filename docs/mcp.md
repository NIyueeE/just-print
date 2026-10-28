# MCP 接入

[MCP](https://modelcontextprotocol.io) 让 Claude Code、Cursor 这类终端里的 agent
直接调工具。Just Print 在容器里内置了 MCP 端点：agent 可以查设备、上传 PDF、
提交打印、跟踪任务。

相关文档：[Web API](api.md) · [部署指南](deployment.md) · [架构与实现](architecture.md)

## 为什么上传要绕一步

网页上传之所以理所当然，是因为**浏览器**跑在你机器上：它读文件、发 POST，字节
直接进服务器。

MCP 里没有浏览器。工具参数是模型生成的文本，如果上传工具收的是文件内容，模型就得
把 PDF 的每个字节写进对话——1 MB 的 PDF 是 34 万 token，而且逐字复述会损坏内容。

所以这里把「控制面」和「数据面」分开：

- **控制面**走 MCP：查设备、提交任务、管任务。
- **数据面**走终端：agent 执行一条 `curl`，把文件从磁盘直接推到服务器。

终端就是 MCP 世界里的浏览器。你不需要额外装任何东西——只要 agent 能跑 shell。

## 配置

配置里有两个显式字段：令牌和设备。

```json
{
  "mcpServers": {
    "just-print-office": {
      "url": "http://192.168.1.10:8080/mcp",
      "headers": {
        "Authorization": "Bearer <JUST_PRINT_TOKEN>",
        "X-Just-Print-Device": "office"
      }
    }
  }
}
```

Claude Code 用命令行添加：

```bash
claude mcp add --transport http just-print-office \
  http://192.168.1.10:8080/mcp \
  --header "Authorization: Bearer <token>" \
  --header "X-Just-Print-Device: office"
```

`X-Just-Print-Device` 是 CUPS 队列名，必须是 `GET /api/printers` 返回的 `id`；
缺这个头直接返回 `400`。想要第二台打印机就再配一个条目——一个连接只对应一台设备，
模型既看不到别的队列，也改不了目标。

令牌只出现在 HTTP 头里，不经过模型上下文，也不会被写进命令行。

## 工具

| 工具 | 参数 | 作用 | 副作用 |
| --- | --- | --- | --- |
| `get_device` | — | 设备状态与 IPP 选项目录 | 无 |
| `create_upload_ticket` | `filename` | 签发一次性上传票据，返回可执行的 curl 命令 | 无 |
| `print_document` | `file_id`、`options?` | 提交打印任务 | **出纸** |
| `list_jobs` | `limit?`、`active?` | 本设备任务列表 | 无 |
| `get_job` | `job_id` | 单个任务 | 无 |
| `cancel_job` | `job_id` | 取消排队/打印中的任务 | 取消任务 |
| `delete_file` | `file_id` | 删除尚未打印的上传文件 | 删临时文件 |

每个工具只做一件事：上传成功不代表打印成功，失败时模型能准确知道停在哪一步。

## 典型流程

用户说「把 `~/reports/q3.pdf` 打出来」：

1. `get_device` —— 确认设备在线，拿到 `sides` / `copies` 等选项的合法取值。
2. `create_upload_ticket({"filename": "q3.pdf"})` —— 返回：

   ```json
   {
     "upload_url": "http://192.168.1.10:8080/uploads/9f1c0b7a...",
     "curl": "curl -sS -X POST --data-binary @{path} 'http://192.168.1.10:8080/uploads/9f1c0b7a...'",
     "expires_in_secs": 300
   }
   ```

3. 在终端执行（把 `{path}` 换成本机路径，必须用 `--data-binary`，`-d` 会去掉换行
   从而损坏 PDF）：

   ```bash
   curl -sS -X POST --data-binary @$HOME/reports/q3.pdf \
     'http://192.168.1.10:8080/uploads/9f1c0b7a...'
   # {"id":"733d6aaf...","name":"q3.pdf","size":482913}
   ```

4. `print_document({"file_id": "733d6aaf...", "options": {"sides": "two-sided-long-edge"}})`
5. 需要跟进度时 `get_job({"job_id": "office-8"})`。

`upload_url` 是从请求的 `Host` 头推出来的；经过反向代理时以代理对外暴露的地址为准。

## 上传票据

票据是这条链路的安全核心：

- **一次性**：兑换即失效，重放返回 `401 invalid_ticket`。
- **5 分钟过期**：没上传就作废。
- **只有一个能力**：创建一个 PDF 文件。不能打印、不能列任务、不能删东西。
- **不绑定令牌**：即使出现在命令行历史或日志里，损失上限也只是「一个 PDF 被创建」。

票据通道只接受 PDF：写入时校验 `%PDF-` 魔数，其它格式返回 `415`，并提示先转换。
转换由 agent 负责，服务端的 LibreOffice 转换仍然只服务 Web 端上传。

## 幂等

打印是物理副作用，所以 `print_document` 会自动生成 `Idempotency-Key`
（由 `file_id`、设备与选项派生）：

- 相同参数在保留期（默认 10 分钟）内重复调用，服务端回放首次结果，
  返回 `replayed: true`，**不会重复出纸**。网络超时后重试属于这种情况。
- 要打第二份不要调两次，用 `options.copies`。
- 重新上传会得到新的 `file_id`，那是一次新的逻辑动作。

## 错误码

工具错误里带服务端错误码和可自纠的提示：

| 错误 | 含义 | 模型该做什么 |
| --- | --- | --- |
| `invalid_ticket` | 票据过期、已被使用或不存在 | 重新调 `create_upload_ticket` |
| `unsupported_media_type` | 上传内容不是 PDF | 先转换成 PDF |
| `payload_too_large` | 超过 `JUST_PRINT_MAX_UPLOAD_BYTES` | 先压缩 |
| `invalid_controls` | 选项不在选项目录里 | 调 `get_device` 取合法值 |
| `printer_unavailable` | 设备已停止或拒绝任务 | 不要重试，告诉用户检查打印机 |
| `service_unavailable` / `gateway_timeout` | CUPS 暂时不可达，附 `Retry-After` | 用相同参数重试 |
| `not_found` | 文件/任务不存在或已过期 | 重新上传 |

## 安全

- 端点与 `/api` 共用同一个 Bearer 令牌，鉴权在进入 MCP 之前完成。
- 设备由请求头 `X-Just-Print-Device` 指定；每个连接只操作那一台设备，`cancel_job` / `get_job` 还会校验
  `job_id` 的设备前缀。
- 服务不启用 CORS，跨站请求拿不到令牌；因此 MCP 端点没有额外做 Origin 白名单
  （反向代理域名千变万化，白名单反而容易配错）。
- 建议给 MCP 和网页端使用同一个令牌的前提下，把服务放在内网或 TLS 网关之后。

## 限制

- 需要 agent 能执行 shell（Claude Code、Cursor agent 可以；纯 GUI 客户端不行）。
- 只接受 PDF ≤ `JUST_PRINT_MAX_UPLOAD_BYTES`（默认 64 MiB）。
- `list_jobs` 在服务端分页之后按设备过滤，任务极多时可能少返回几条；
  需要精确结果时用 `get_job`。
- 同时缓存的设备端点上限 32；超出返回 `400`（正常部署远达不到）。
