/* 展示层格式化工具（无副作用、可单测）。 */

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) {
    return '—'
  }
  if (bytes < 1024) {
    return `${bytes} B`
  }
  if (bytes < 1024 * 1024) {
    return `${(bytes / 1024).toFixed(1)} KiB`
  }
  if (bytes < 1024 * 1024 * 1024) {
    return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`
  }
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GiB`
}

export function formatRelativeTime(ms: number, now: number = Date.now()): string {
  if (!Number.isFinite(ms) || ms <= 0) {
    return '等待状态更新…'
  }
  const elapsed = now - ms
  if (elapsed < 60_000) {
    return '刚刚提交'
  }
  if (elapsed < 3_600_000) {
    return `${Math.floor(elapsed / 60_000)} 分钟前提交`
  }
  const date = new Date(ms)
  const sameDay = date.toDateString() === new Date(now).toDateString()
  const time = date.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' })
  if (sameDay) {
    return `${time} 提交`
  }
  return `${date.getMonth() + 1}月${date.getDate()}日 ${time} 提交`
}

export function formatAbsoluteTime(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) {
    return '未知时间'
  }
  return new Date(ms).toLocaleString('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  })
}

export function formatDurationMs(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) {
    return '稍后'
  }
  const seconds = Math.ceil(ms / 1000)
  if (seconds < 60) {
    return `${seconds} 秒`
  }
  return `${Math.ceil(seconds / 60)} 分钟`
}
