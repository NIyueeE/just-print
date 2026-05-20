"""
Just-Printing-Server: 极简打印中继服务
FastAPI后端主文件

技术约束：
- 无数据库、无用户体系、无持久化存储
- 所有文件仅存内存或临时目录，请求结束立即销毁
- 所有日志仅输出到 stdout
- 所有配置通过环境变量注入

API端点：
- POST /auth: 认证
- POST /upload: 文件上传
- GET /preview.pdf: PDF预览
- GET /printer/status: 打印机状态
- POST /print: 打印投递
- POST /cancel: 取消会话
"""

import os
import asyncio
from contextlib import asynccontextmanager

from fastapi import FastAPI

from .config import config
from .services import cleanup_sessions
from .routes import create_routes
from .logging_config import setup_logging, get_logger

# 配置日志
setup_logging()
logger = get_logger(__name__)


# 后台任务：每5分钟清理一次过期会话
async def session_cleanup_loop():
    """后台任务：每5分钟清理一次过期会话"""
    while True:
        await asyncio.sleep(300)  # 5分钟
        cleaned = cleanup_sessions()
        if cleaned > 0:
            logger.info(f"后台清理: 已清理 {cleaned} 个过期会话")


# 创建FastAPI应用（带生命周期管理）
@asynccontextmanager
async def lifespan(app: FastAPI):
    # 启动时：启动后台任务清理过期会话
    cleanup_task = asyncio.create_task(session_cleanup_loop())
    yield
    # 关闭时：取消后台任务
    cleanup_task.cancel()
    try:
        await cleanup_task
    except asyncio.CancelledError:
        pass
    # 关闭全局 aiohttp session
    if hasattr(app.state, 'http_session') and app.state.http_session:
        await app.state.http_session.close()


app = FastAPI(
    title="Just-Printing-Server",
    description="极简打印中继服务",
    version="1.0.0",
    lifespan=lifespan
)


# 注册所有路由
create_routes(app)


if __name__ == "__main__":
    import uvicorn
    uvicorn.run(app, host=config.HOST, port=config.PORT)
