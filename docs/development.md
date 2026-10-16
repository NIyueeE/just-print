# 本地开发与检查

项目使用 [just](https://github.com/casey/just) 统一本地检查链；需要 Rust、Bun
与 `just`，容器构建需要 Podman 或 Docker。

## 常用命令

| 命令 | 说明 |
| --- | --- |
| `just` / `just check` | 后端 + 前端完整检查 |
| `just backend` | 格式检查（fmt）+ 静态检查（clippy）+ 测试 |
| `just frontend` | 依赖校验（frozen-lockfile）+ 类型检查 + lint + 格式 + 单元测试 + 生产构建 |
| `just shell` | 入口脚本语法检查 + PPD 匹配/选项解析的纯逻辑测试 |
| `just container` | 用 podman 或 docker 构建本地镜像 |
| `just debug` | 构建并以 `JUST_PRINT_CUPS_PDF=1` 前台运行调试容器（需先设置 `JUST_PRINT_TOKEN`） |
| `cd frontend && bun run dev` | 启动 Vite 开发服务器 |

开发服务器会把 `/api` 代理到 `http://localhost:8080`，联调接口时先启动后端
（`cargo run` 或 `just debug`）并设置 `JUST_PRINT_TOKEN`。

## 门禁分层：轻门 / 重门

本地 git 钩子与远端 CI 复用**同一套入口脚本**（`scripts/` 目录），不会出现
「本地过了、CI 挂了」或反之的漂移。分层的目的是把快的检查留在本地：

| 层 | 入口 | 内容 | 触发时机 |
| --- | --- | --- | --- |
| 轻门 | `just gate-light` → `scripts/gate-light.sh` | 仓库卫生、typos、`cargo fmt --check`、prettier、eslint、shellcheck、入口逻辑测试、actionlint、hadolint、zizmor | 每次 `git commit`（pre-commit） |
| 重门 | `just gate-heavy` → `scripts/gate-heavy.sh` | `just check`（fmt + clippy + cargo test + 前端构建）+ `scripts/smoke.sh` 容器冒烟 | 每次 `git push`（pre-push） |

```bash
# 手动跑某一层
just gate-light
just gate-heavy
```

环境变量：

| 变量 | 说明 |
| --- | --- |
| `PROBE=strict` | 工具缺失视为失败（CI 里设置；本地缺工具只打 `skip`） |
| `JUST_SKIP_HOOKS=1` | 跳过本次钩子（应急用，勿用于日常） |
| `JUST_HOOK_LIGHT=0` | 本次提交改用重门（等价 pre-push 的强度） |

### 本地需要的工具

| 工具 | 轻门用途 | 缺失时 |
| --- | --- | --- |
| `just`、`cargo`、`bun` | 必需 | 钩子直接报错 |
| `typos` | 拼写检查 | 本地跳过，CI 失败 |
| `shellcheck` | 入口脚本静态检查 | 本地跳过，CI 失败 |
| `actionlint` | workflow lint | 本地跳过，CI 失败 |
| `hadolint` | Containerfile lint | 本地跳过，CI 失败 |
| `zizmor` | workflow 安全审计 | 本地跳过，CI 失败 |
| `podman` 或 `docker` | 重门冒烟 | 本地跳过，CI 失败 |

安装示例（二进制直装，无需包管理器）：

```bash
mkdir -p ~/.local/bin
# typos
curl -fsSL "$(curl -fsSL https://api.github.com/repos/crate-ci/typos/releases/latest \
  | jq -r .tag_name | sed 's|^|https://github.com/crate-ci/typos/releases/download/|')" \
  | tar -xz -C ~/.local/bin ./typos
# shellcheck / actionlint / hadolint 同理（见各自 release 资产名）
uv tool install zizmor   # 或 pip install --user zizmor
```

## 提交钩子

首次克隆后执行一次以下命令，即可启用钩子（pre-commit 跑轻门、pre-push 跑
重门、commit-msg 校验提交信息约定）：

```bash
just init-hooks
```

钩子脚本位于 `.githooks/`，随仓库一起维护；检查失败时提交/推送会被中止。
卸载用 `just remove-hooks`。

设计意图是**本地先拦一道**：轻门几十秒内反馈格式/拼写/ lint 问题，重门在
推送前把完整构建、单元测试与容器端到端都跑掉；远端 CI 再跑同一套作最终兜底
（见 [CI 门禁](#ci-门禁)）。想临时越过门禁时用 `JUST_SKIP_HOOKS=1 git commit`，
但请在提交信息里说明原因。

## CI 门禁

`.github/workflows/ci.yml` 与本地共用 `scripts/` 下的入口脚本：

| CI 作业 | 等价本地命令 |
| --- | --- |
| `Lint (gate-light)` | `PROBE=strict just gate-light` |
| `Check (gate-heavy …)` | `PROBE=strict scripts/gate-heavy.sh --no-smoke` |
| `Smoke (gate-heavy …)` | `PROBE=strict scripts/smoke.sh` |
| `Dependency audit` | `just audit`（Rust advisory + license、前端 npm） |
| `Build image` | `just container` + Trivy 镜像漏洞扫描 |
| `Validate compose example` | `docker compose -f examples/compose.yaml config` |
| `Gate summary` | 汇总作业：分支保护只勾选它即可覆盖全部门禁 |

`release.yml` 以**上游模式**（`uses: ./.github/workflows/ci.yml`）复用同一份
CI：推 tag 时先跑完整门禁，通过后才构建推送镜像并创建 GitHub Release，
不会出现「tag 绕过检查链直接发布」。

## 代码组织

- Rust 后端：handlers 位于 `src/api/`，CUPS 集成位于 `src/cups/`，文档转换位于
  `src/conversion.rs`，临时文件管理位于 `src/store.rs`。
- Preact 前端：可复用 UI 位于 `frontend/src/components/`，API 类型与调用位于
  `frontend/src/api.ts`。

更多实现细节见 [architecture.md](architecture.md)。
## 前端视觉检查（无容器环境）

宿主机没有 CUPS / LibreOffice 时，真实后端无法提供可用数据（`/api/printers`
返回 `503 service_unavailable`，`/api/files` 转换返回 `422`）。可用
「Vite + mock API + 无头浏览器」在本地完整走查前端各状态，无需改动任何项目文件。

### 1. 启动 Vite

务必显式绑定 IPv4：Vite 默认只监听 `localhost`（可能解析为 `::1`），无头
浏览器访问 `127.0.0.1` 会得到 `ERR_CONNECTION_REFUSED`，截图出来是浏览器
错误页而不是应用。

```bash
cd frontend && bun run dev -- --host 127.0.0.1 --port 5173 --strictPort
```

### 2. 提供 API 数据

- 有真实后端：`cargo run`（需先 `export JUST_PRINT_TOKEN=...`）。
- 无 CUPS / LibreOffice：写一个 mock API 跑在 Vite 代理目标
  `localhost:8080`（绑定 `0.0.0.0`，避免 `localhost` 解析为 IPv6）。响应
  须与 [api.md](api.md) 契约一致（统一错误信封、`201/202` 状态码、字段名、
  IPP 选项 `kind` 联合类型、`JobView` 字段）。
- 需要 mock 的端点：`GET /api/formats`、`GET /api/printers`、
  `POST /api/files`、`GET /api/files/{id}`、`POST /api/print`（回显
  `Idempotency-Key` 行为）、`GET /api/jobs`、`GET /api/jobs/{id}`、
  `DELETE /api/jobs/{id}`。
- 截取任务状态流转：前端轮询 `GET /api/jobs?active=true`，mock 按调用次数依次
  返回 `queued → printing → completed` 即可截到不同状态。
- 预览接口（`GET /api/files/{id}`）返回带 `Content-Length` 的合法 PDF。
  最小 PDF 也要正确计算 `xref` 偏移和流 `Length`，否则 iframe 预览渲染失败。

### 3. 截图

静态页面用 Chromium 命令行即可：

```bash
chromium --headless=new --disable-gpu --no-sandbox --hide-scrollbars \
  --window-size=1280,900 --virtual-time-budget=8000 \
  --screenshot=/tmp/shot.png http://127.0.0.1:5173/
```

需要交互（注入令牌、上传、点击）时用 Playwright（`playwright-core` +
本机 Chromium 可执行文件），注意：

- 令牌存于 `sessionStorage`，键为 `just_print_token`（见
  `frontend/src/api.ts` 的 `TOKEN_KEY`）；注入后需 `reload`。登录页会先用
  `GET /api/printers` 校验令牌，mock 需要让该请求成功。
- 上传走 `#file-input` 的 `setInputFiles`，转换完成以
  `.preview iframe` 出现为标志；`waitForSelector` 时给足超时。
- 预览 iframe 很高，任务状态区常在折叠线以下：视口截图会漏掉，需
  `fullPage: true` 或先 `scrollTo` 到底部。
- 未上传文档时「提交打印」按钮置灰，先完成上传再点；按钮按文案定位时用
  `hasText` 而非精确文本（按钮内还包含图标）。

### 4. 分析截图

用 imageread 技能逐张分析（UI 小字用 `--mode ocr --budget large`），核对
截图文案与 `frontend/src/components/` 中的实际代码是否一致。若分析结果是
浏览器错误页（如 `ERR_CONNECTION_REFUSED`），先检查服务监听地址与代理目标，
再重新截图；两张截图字节完全相同说明页面没变化，优先怀疑截图时机或折叠线
以下内容，而不是分析工具。

### 5. 清理

结束后停掉 Vite 与 mock 进程；截图保留在 `/tmp` 即可，不进入仓库。

