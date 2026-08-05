# 本地开发与检查

项目使用 [just](https://github.com/casey/just) 统一本地检查链；需要 Rust、Bun
与 `just`，容器构建需要 Podman 或 Docker。

## 常用命令

| 命令 | 说明 |
| --- | --- |
| `just` / `just check` | 后端 + 前端完整检查 |
| `just backend` | 格式检查（fmt）+ 静态检查（clippy）+ 测试 |
| `just frontend` | 依赖校验（frozen-lockfile）+ 类型检查 + 生产构建 |
| `just container` | 用 podman 或 docker 构建本地镜像 |
| `just debug` | 构建并以 `JUST_PRINT_CUPS_PDF=1` 前台运行调试容器（需先设置 `JUST_PRINT_TOKEN`） |
| `cd frontend && bun run dev` | 启动 Vite 开发服务器 |

开发服务器会把 `/api` 代理到 `http://localhost:8080`，联调接口时先启动后端
（`cargo run` 或 `just debug`）并设置 `JUST_PRINT_TOKEN`。

## 提交钩子

首次克隆后执行一次以下命令，即可让每次 `git commit` 前自动运行 `just check`：

```bash
just init-hooks
```

钩子脚本位于 `.githooks/pre-commit`，随仓库一起维护；检查失败时提交会被中止。

## 代码组织

- Rust 后端：handlers 位于 `src/api/`，CUPS 集成位于 `src/cups/`，文档转换位于
  `src/conversion.rs`，临时文件管理位于 `src/store.rs`。
- Preact 前端：可复用 UI 位于 `frontend/src/components/`，API 类型与调用位于
  `frontend/src/api.ts`。

更多实现细节见 [architecture.md](architecture.md)。
