/* 通用轮询 hook：带在途守卫、AbortController、指数退避与页面隐藏暂停。 */

import { useEffect, useRef } from 'preact/hooks'

export interface PollingOptions {
  /** 正常轮询间隔。 */
  intervalMs: number
  /** 是否启用轮询（例如没有进行中的任务时可以关闭）。 */
  enabled: boolean
  /** 首次失败前的延迟（毫秒）。 */
  backoffBaseMs?: number
  /** 退避上限。 */
  backoffMaxMs?: number
  /** 页面不可见时暂停，恢复可见时立刻执行一次。 */
  pauseWhenHidden?: boolean
}

/** 读取 `ApiError.retryAfterMs`（鸭子类型，避免轮询与具体客户端耦合）。 */
function readRetryAfterMs(error: unknown): number {
  if (typeof error === 'object' && error !== null && 'retryAfterMs' in error) {
    const value = (error as { retryAfterMs?: unknown }).retryAfterMs
    if (typeof value === 'number' && Number.isFinite(value) && value > 0) {
      return value
    }
  }
  return 0
}

/** `task` 抛错即视为一次失败，hook 会按指数退避安排下一次执行。 */
export function usePolling(
  task: (signal: AbortSignal) => Promise<unknown>,
  options: PollingOptions,
): void {
  const {
    intervalMs,
    enabled,
    backoffBaseMs = 1000,
    backoffMaxMs = 30_000,
    pauseWhenHidden = true,
  } = options

  const taskRef = useRef(task)
  taskRef.current = task

  const inFlightRef = useRef(false)
  const failuresRef = useRef(0)
  const timerRef = useRef<number | null>(null)
  const controllerRef = useRef<AbortController | null>(null)

  useEffect(() => {
    if (!enabled) {
      return
    }
    let disposed = false

    const clearTimer = (): void => {
      if (timerRef.current !== null) {
        window.clearTimeout(timerRef.current)
        timerRef.current = null
      }
    }

    const schedule = (delayMs: number): void => {
      if (disposed) {
        return
      }
      clearTimer()
      timerRef.current = window.setTimeout(() => {
        void run()
      }, delayMs)
    }

    const run = async (): Promise<void> => {
      if (disposed || inFlightRef.current) {
        return
      }
      if (pauseWhenHidden && document.hidden) {
        schedule(intervalMs)
        return
      }
      inFlightRef.current = true
      const controller = new AbortController()
      controllerRef.current = controller
      try {
        await taskRef.current(controller.signal)
        failuresRef.current = 0
        schedule(intervalMs)
      } catch (error) {
        if (disposed || controller.signal.aborted) {
          return
        }
        failuresRef.current += 1
        const backoff = Math.min(backoffBaseMs * 2 ** (failuresRef.current - 1), backoffMaxMs)
        // 503/409 响应会带 Retry-After（ApiError.retryAfterMs）：至少等待服务端建议的时长。
        schedule(Math.max(backoff, readRetryAfterMs(error)))
      } finally {
        inFlightRef.current = false
        if (controllerRef.current === controller) {
          controllerRef.current = null
        }
      }
    }

    const onVisibilityChange = (): void => {
      if (!document.hidden && !inFlightRef.current) {
        clearTimer()
        void run()
      }
    }

    schedule(0)
    if (pauseWhenHidden) {
      document.addEventListener('visibilitychange', onVisibilityChange)
    }
    return () => {
      disposed = true
      clearTimer()
      controllerRef.current?.abort()
      controllerRef.current = null
      inFlightRef.current = false
      if (pauseWhenHidden) {
        document.removeEventListener('visibilitychange', onVisibilityChange)
      }
    }
  }, [enabled, intervalMs, backoffBaseMs, backoffMaxMs, pauseWhenHidden])
}
