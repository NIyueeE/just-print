# Just Printing Server

中文 | [English](README.md)

一个极简、无状态的打印中继服务，基于 FastAPI 构建。

## 特性

- **零持久化**: 无数据库、无用户体系、无持久化存储
- **纯内存处理**: 所有文件仅存于内存或临时目录，请求结束后立即销毁
- **环境变量配置**: 所有配置通过环境变量注入
- **IPP 协议**: 直接连接支持 IPP 协议的打印机
- **限流保护**: 内置基于 IP 的速率限制
- **Podman 就绪**: 支持 Podman Compose 一键部署

## 使用场景

### 无线连接家庭打印机
将您的家庭打印机直接连接到网络。Just-Printing-Server 支持任何 IPP 协议的打印机：
- **HP 打印机**（如 HP LaserJet、HP OfficeJet）
- **Brother 打印机**（如 Brother HL、MFC 系列）
- **Canon 打印机**（如 Canon PIXMA、imageRUNNER）
- **EPSON 打印机**（如 EPSON WorkForce、EcoTank）
- **联想打印机**（如联想 LJ4000D、LJ2600D）

无需在手机或电脑上安装任何打印机驱动或应用。

### 轻量部署
可部署在以下设备上：
- 树莓派（3B+/4/5）
- NAS 设备（支持 Docker/Podman）
- 迷你主机
- 任何 Linux 服务器

### 工作原理
1. 您的打印机已连接 WiFi 并拥有 IPP 地址
2. 在同一网络下的设备上部署 Just-Printing-Server
3. 从任何设备（手机、平板、笔记本）访问 Web 界面
4. 上传照片或 PDF，调整设置，点击打印 - 就是这么简单！

## 项目结构

```
├── backend/                 # FastAPI 后端
│   ├── __init__.py
│   ├── main.py             # 应用入口
│   ├── config.py           # 配置（从 .env 读取）
│   ├── models.py           # Pydantic 数据模型
│   ├── logging_config.py   # 日志配置
│   ├── routes.py           # API 端点
│   └── services/
│       ├── session.py      # 会话管理
│       ├── auth.py         # 认证
│       ├── file_handler.py # 文件验证与 PDF 转换
│       └── printer.py      # IPP 打印机交互
├── frontend/               # Web 界面（原生 JS）
│   ├── index.html
│   ├── app.js
│   └── styles.css
├── docker/                 # 部署——生产只需此目录
│   ├── Dockerfile          # 多阶段构建（git clone + pip install）
│   ├── compose.yaml        # Podman Compose 编排
│   ├── .env.example        # 配置模板
│   └── justfile            # 快捷命令
├── .env.example -> docker/.env.example  # 软链接，方便本地开发
├── pyproject.toml          # 项目元数据和依赖
├── uv.lock                 # 锁文件（可复现安装）
├── CLAUDE.md               # AI 编程助手指引
└── LICENSE                 # MIT
```

## 快速开始

### 方式一：使用 Podman Compose 部署（生产）

将 `docker/` 目录拷贝到目标机器，配置 `.env` 后启动：

```bash
cd docker
cp .env.example .env
# 编辑 .env 配置你的打印机信息

podman-compose up -d
```

### 方式二：使用 uv 启动（本地开发）

```bash
# 如果未安装 uv，先安装它
curl -LsSf https://astral.sh/uv/install.sh | sh

# 初始化环境
cp docker/.env.example .env
# 编辑 .env 配置你的打印机信息
uv sync

# 运行服务
uv run python backend/main.py
```

### 3. 访问 Web 界面

在浏览器中打开 `http://localhost:3001`。

Web 界面功能：
- **拖拽上传文件** - 支持 JPG、PNG 和 PDF 文件
- **PDF 预览** - 打印前预览合并后的文档
- **打印设置** - 份数、单双面、色彩、纸张尺寸、质量、方向
- **打印机状态** - 实时显示打印机在线/离线状态
- **移动端适配** - 在手机和平板上也能完美使用

## API 接口

| 方法 | 接口 | 说明 |
|------|------|------|
| POST | `/auth` | 认证 |
| POST | `/upload` | 上传文件（图片/PDF） |
| GET | `/preview.pdf` | 预览合并后的 PDF |
| GET | `/printer/status` | 查询打印机状态 |
| GET | `/printer/capabilities` | 获取打印机能力（支持的打印选项） |
| POST | `/print` | 提交打印任务 |
| POST | `/cancel` | 取消会话 |

## 环境变量

| 变量名 | 默认值 | 说明 |
|--------|--------|------|
| `REGISTRY` | docker.io | Docker 镜像仓库（用于 compose） |
| `FRONTEND_PATH` | frontend | 前端静态文件路径 |
| `PRINTER_IPP_URL` | - | 打印机的 IPP 地址 |
| `PRINTER_NAME` | - | 打印机显示名称 |
| `ACCESS_TOKEN` | - | API 访问令牌 |
| `MAX_UPLOAD_MB` | 50 | 最大上传大小（MB） |
| `RATE_LIMIT_PER_IP` | 5/minute | 每 IP 限流频率 |
| `LOG_LEVEL` | INFO | 日志级别 |
| `IPP_DEFAULT_MEDIA` | iso_a4_210x297mm | 默认纸张尺寸 |
| `IPP_DEFAULT_QUALITY` | normal | 默认打印质量 |
| `IPP_DEFAULT_ORIENTATION` | portrait | 默认打印方向 |
| `IPP_USER_NAME` | fastapi | 打印任务用户名 |
| `HOST` | 127.0.0.1 | 服务监听地址 |
| `PORT` | 3001 | 服务端口 |

## 打印机能力检测

`/printer/capabilities` 接口会自动检测打印机支持的打印选项。在 Web 界面中：
- 支持的选项显示为可点击的按钮
- 不支持的选项显示为灰色并禁用
- 用户可以清楚了解其打印机支持哪些设置

## 技术栈

- **后端**: FastAPI + Python 3.11
- **打印**: IPP 协议（`pyipp`）
- **PDF 处理**: PyPDF2, img2pdf, Pillow
- **限流**: slowapi
- **部署**: Podman + Podman Compose
- **包管理**: uv（带锁文件）

## 许可证

MIT
