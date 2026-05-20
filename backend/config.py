"""
配置管理模块
"""

import os
from dotenv import load_dotenv
load_dotenv()


class Config:
    """应用配置类"""
    FRONTEND_PATH = os.getenv("FRONTEND_PATH", "frontend")
    PRINTER_IPP_URL = os.getenv("PRINTER_IPP_URL", "")
    PRINTER_NAME = os.getenv("PRINTER_NAME", "Unknown Printer")
    ACCESS_TOKEN = os.getenv("ACCESS_TOKEN", "")
    MAX_UPLOAD_MB = int(os.getenv("MAX_UPLOAD_MB", "50"))
    RATE_LIMIT_PER_IP = os.getenv("RATE_LIMIT_PER_IP", "5/minute")
    LOG_LEVEL = os.getenv("LOG_LEVEL", "INFO")

    # IPP 打印配置
    IPP_DEFAULT_MEDIA = os.getenv("IPP_DEFAULT_MEDIA", "iso_a4_210x297mm")
    IPP_DEFAULT_QUALITY = os.getenv("IPP_DEFAULT_QUALITY", "normal")
    IPP_DEFAULT_ORIENTATION = os.getenv("IPP_DEFAULT_ORIENTATION", "portrait")
    IPP_USER_NAME = os.getenv("IPP_USER_NAME", "fastapi")

    # 服务监听配置
    HOST = os.getenv("HOST", "127.0.0.1")
    PORT = int(os.getenv("PORT", "3001"))


config = Config()
