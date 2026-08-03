# Just Print — Web API v1

除 `/healthz` 与静态前端外，所有接口都在 `/api` 前缀下，并且必须携带
`Authorization: Bearer <token>`（`JUST_PRINT_TOKEN`）。未带令牌或令牌错误返回
`401`；令牌未配置时服务拒绝启动（fail-closed）。

## 通用错误格式

所有错误响应均为 JSON：

```json
{
  "error": {
    "code": "not_found",
    "message": "服务可能已重启，请重新上传文件"
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
| `printer_unavailable` | 409 | 打印机不存在、不支持 PDF 或未就绪 |
| `invalid_controls` | 400 | 控制信息不是能力查询返回的合法值 |
| `internal` | 500 | 内部错误 |

## POST /api/files

`multipart/form-data`，字段名为 `file`，最大 64 MiB。

支持扩展名：`.pdf`、`.docx`、`.xlsx`、`.pptx`、`.odt`、`.ods`、`.odp`、`.md`、
`.txt`。`.pdf` 校验后原样通过；其它格式由镜像内置 LibreOffice 转换为 PDF。

成功（`201 Created`）：

```json
{
  "id": "1234567890abcdef1234567890abcdef",
  "name": "report.docx",
  "size": 12345
}
```

失败：`400`（缺少文件字段）、`413`（过大）、`415`（格式不支持）、
`422`（转换失败）。

## GET /api/files/{id}

返回转换后的 PDF 字节流，`Content-Type: application/pdf`，
`Content-Disposition: inline; filename="preview.pdf"`。文件不存在返回 `404`。

## GET /api/printers

成功（`200`）：

```json
{
  "printers": [
    {
      "id": "serial-or-lp-path",
      "name": "HP LaserJet M203dw",
      "manufacturer": "HP",
      "serial": "CN12345678",
      "pdf_supported": true,
      "capabilities": {
        "DUPLEX": {
          "default": "OFF",
          "kind": "enumerated",
          "values": ["OFF", "ON"]
        },
        "BINDING": {
          "default": "LONGEDGE",
          "kind": "enumerated",
          "values": ["LONGEDGE", "SHORTEDGE"]
        },
        "DENSITY": {
          "default": "0",
          "kind": "range",
          "min": -6,
          "max": 6
        }
      }
    }
  ]
}
```

- `capabilities` 为 `null` 表示能力尚未查询到（查询失败会在后台自动重试）。
- `pdf_supported` 仅在能力已知时有意义：PJL `PERSONALITY` 不含 `PDF` 的打印机
  会显示为 `false`，提交打印将被拒绝。
- 能力字段只暴露实用参数：`DUPLEX`、`BINDING`、`ECONOMODE`、`DENSITY`、
  `MEDIATYPE`、`RESOLUTION`；`PERSONALITY` 仅用于判断 `pdf_supported`，不暴露给
  前端。

## POST /api/print

请求体：

```json
{
  "file_id": "1234567890abcdef1234567890abcdef",
  "printer_id": "serial-or-lp-path",
  "controls": {
    "DUPLEX": "ON",
    "BINDING": "LONGEDGE"
  }
}
```

`controls` 可省略；提供时每个键必须来自上述六个实用参数，值必须是能力查询
返回的合法值（枚举值或范围），否则 `400 invalid_controls`。

成功（`202 Accepted`）：

```json
{
  "job_id": "abcdef1234567890abcdef1234567890"
}
```

失败：`404 not_found`（文件或打印机不存在）、`409 printer_unavailable`
（不支持 PDF / 能力未知 / 打印机已移除）、`400 invalid_controls`。

## GET /api/jobs/{id}

成功（`200`）：

```json
{
  "id": "abcdef1234567890abcdef1234567890",
  "printer_id": "serial-or-lp-path",
  "status": "queued",
  "error": null,
  "created_at_ms": 1783173600000
}
```

`status`：`queued` / `printing` / `success` / `failed`。失败时 `error` 为人类可读
原因。任务只存在于内存，服务重启后一律返回 `404`，前端统一提示
「服务已重启，请重新上传」。
