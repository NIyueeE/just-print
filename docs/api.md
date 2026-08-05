# Just Print — Web API v1

相关文档：[部署指南](deployment.md) · [架构与实现](architecture.md) ·
[本地开发](development.md)

除 `/healthz` 与静态前端外，所有接口都在 `/api` 前缀下，并且必须携带
`Authorization: Bearer <token>`（`JUST_PRINT_TOKEN`）。未带令牌或令牌错误返回
`401`；令牌未配置时服务拒绝启动（fail-closed）。

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
| `printer_unavailable` | 409 | CUPS 拒绝提交（打印机停止/禁用等） |
| `invalid_controls` | 400 | 选项不是打印机选项查询返回的合法值 |
| `internal` | 500 | 内部错误或 CUPS 不可用 |

## POST /api/files

`multipart/form-data`，字段名为 `file`，最大 64 MiB。

支持扩展名（与镜像内置 LibreOffice 7.4.7 注册的 `IMPORT` 过滤器一致，共 172 个）：
`.123`、`.602`、`.abw`、`.bmp`、`.cdr`、`.cgm`、`.cmx`、`.csv`、`.cwk`、
`.dbf`、`.dif`、`.doc`、`.docm`、`.docx`、`.dot`、`.dotm`、`.dotx`、`.dps`、
`.dpt`、`.dxf`、`.emf`、`.emz`、`.eps`、`.et`、`.ett`、`.fb2`、`.fh`、`.fh1`、
`.fh10`、`.fh11`、`.fh2`、`.fh3`、`.fh4`、`.fh5`、`.fh6`、`.fh7`、`.fh8`、
`.fh9`、`.fodg`、`.fodp`、`.fods`、`.fodt`、`.gif`、`.gnm`、`.gnumeric`、
`.htm`、`.html`、`.hwp`、`.jfif`、`.jif`、`.jpe`、`.jpeg`、`.jpg`、`.key`、
`.lrf`、`.lwp`、`.mcw`、`.md`、`.met`、`.mov`、`.mp`、`.mw`、`.mwd`、
`.numbers`、`.nx^d`、`.odc`、`.odg`、`.odm`、`.odp`、`.ods`、`.odt`、`.otg`、
`.oth`、`.otm`、`.otp`、`.ots`、`.ott`、`.p65`、`.pages`、`.pbm`、`.pcd`、
`.pct`、`.pcx`、`.pdb`、`.pdf`、`.pgm`、`.pict`、`.pm`、`.pm6`、`.pmd`、
`.png`、`.pot`、`.potm`、`.potx`、`.ppm`、`.pps`、`.ppsx`、`.ppt`、`.pptm`、
`.pptx`、`.psd`、`.psw`、`.pub`、`.qxd`、`.qxt`、`.ras`、`.rtf`、`.sda`、
`.sdc`、`.sdd`、`.sdw`、`.slk`、`.stc`、`.std`、`.sti`、`.stw`、`.svg`、
`.svgz`、`.svm`、`.sxc`、`.sxd`、`.sxg`、`.sxi`、`.sxs`、`.sxw`、`.sylk`、
`.tab`、`.tga`、`.tif`、`.tiff`、`.tsv`、`.txt`、`.vdx`、`.vsd`、`.vsdm`、`.vsdx`、
`.wb1`、`.wb2`、`.wdb`、`.webp`、`.wk1`、`.wk3`、`.wk4`、`.wks`、`.wmf`、
`.wmz`、`.wn`、`.wpd`、`.wpg`、`.wps`、`.wpt`、`.wq1`、`.wq2`、`.wri`、
`.xbm`、`.xhtml`、`.xlc`、`.xlk`、`.xlm`、`.xls`、`.xlsb`、`.xlsm`、`.xlsx`、
`.xlt`、`.xltm`、`.xltx`、`.xlw`、`.xml`、`.xpm`、`.zabw`、`.zip`、`.zmf`。
`.pdf` 校验后原样通过；`.md` 先渲染为带打印样式的 HTML（原始 HTML 一律转义）
再转换；其它格式由镜像内置 LibreOffice 转换为 PDF。

成功（`201 Created`）：

```json
{
  "id": "1234567890abcdef1234567890abcdef",
  "name": "report.docx",
  "size": 12345
}
```

失败：`400`（缺少文件字段或文件为空）、`413`（过大）、`415`（格式不支持）、
`422`（转换失败）。

## GET /api/files/{id}

流式返回转换后的 PDF（带 `Content-Length`），`Content-Type: application/pdf`，
`Content-Disposition: inline; filename="preview.pdf"`。文件不存在返回 `404`。

## GET /api/printers

成功（`200`）：

```json
{
  "printers": [
    {
      "id": "CUPS-PDF",
      "name": "CUPS-PDF Printer",
      "state": "idle",
      "options": {
        "PageSize": {
          "default": "A4",
          "kind": "enumerated",
          "values": ["Letter", "A4", "A5"]
        },
        "Duplex": {
          "default": "None",
          "kind": "enumerated",
          "values": ["DuplexTumble", "DuplexNoTumble", "None"]
        },
        "copies": {
          "default": null,
          "kind": "range",
          "min": 1,
          "max": 9999
        }
      }
    }
  ]
}
```

- `id` 是 CUPS 打印机名，也是提交任务时的 `printer_id`。
- `state` 来自 `lpstat -p -l`：`idle` / `printing` / `disabled` / `stopped`。
- `options` 来自 `lpoptions -p <printer> -l`，只暴露 CUPS/PPD 提供的合法控制项；
  范围选项只有 `min` / `max`，没有 `values`。
- 打印机没有可用选项（如 raw 队列）时 `options` 为空对象。

## POST /api/print

请求体：

```json
{
  "file_id": "1234567890abcdef1234567890abcdef",
  "printer_id": "CUPS-PDF",
  "options": {
    "PageSize": "A4",
    "Duplex": "None",
    "copies": "2"
  }
}
```

`options` 可省略；提供时每个键必须来自 `/api/printers` 返回的选项表，值必须是
该选项的枚举值或范围值，否则 `400 invalid_controls`。

成功（`202 Accepted`）：

```json
{
  "job_id": "CUPS-PDF-8"
}
```

`job_id` 是 CUPS 任务 id（`printer-job` 形式）。

失败：`404 not_found`（文件或打印机不存在）、`400 invalid_controls`、
`409 printer_unavailable`（CUPS 拒绝提交）。

任务提交给 CUPS 后由 CUPS 排队、执行与跟踪；打印语言由 CUPS 过滤链根据驱动 /
PPD 决定，客户端不需要指定。

## GET /api/jobs/{id}

`{id}` 接受 `printer-job`（如 `CUPS-PDF-8`）或纯数字 CUPS 任务号。

成功（`200`）：

```json
{
  "id": "CUPS-PDF-8",
  "printer_id": "CUPS-PDF",
  "status": "printing",
  "error": null,
  "created_at_ms": 1783173600000
}
```

`status`：`queued` / `printing` / `completed` / `failed` / `canceled`。失败或取消
时 `error` 为 CUPS 提供的 `job-state-reasons`；任务在 CUPS spool 中已被清理时
返回 `404`。
