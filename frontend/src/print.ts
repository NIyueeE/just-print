/* =============================================================================
 * 打印提交编排：幂等键复用、自动重试、错误归类、上下文更新。
 *
 * 与 UI 解耦，便于单测；组件只调用 `submit(pending)`。
 * ========================================================================== */

import { useCallback, useRef } from 'preact/hooks'
import {
  ApiError,
  errorMessage,
  isAbortError,
  isIdempotencyConflict,
  isRetryable,
  submitPrint,
  type JobView,
  type PrintResponse,
} from './api'
import { useAuthGuard } from './hooks/useAuthGuard'
import { printFingerprint, resolveAttempt } from './idempotency'
import { createNotice, useAppDispatch, useAppState, type PendingPrint } from './state'

/** 503/504/网络错误与 409 幂等冲突的自动重试上限。 */
export const MAX_SUBMIT_RETRIES = 3

const MAX_BACKOFF_MS = 8000

export function friendlyPrintError(error: unknown): string {
  if (error instanceof ApiError) {
    switch (error.code) {
      case 'not_found':
        return '文件或打印机已不存在，请重新上传文档或重新选择打印机。'
      case 'invalid_controls':
        return `打印选项未被打印机接受：${error.message}`
      case 'printer_unavailable':
        return '打印机当前不可用（已停止或拒绝新任务）。'
      case 'payload_too_large':
        return '文件超过服务器允许的大小。'
      case 'conversion_failed':
        return '文档转换失败，请确认文件可以正常打开。'
      case 'unsupported_media_type':
        return '不支持的文件格式。'
      case 'bad_gateway':
        return '打印服务与 CUPS 通信异常，请稍后重试。'
      case 'service_unavailable':
        return 'CUPS 暂时不可用，请稍后重试。'
      case 'gateway_timeout':
        return 'CUPS 响应超时，请稍后重试。'
      case 'idempotency_conflict':
        return '该打印请求仍在处理中，请稍候重试。'
      case 'internal':
        return `服务内部错误：${error.message}`
      default:
        return error.message
    }
  }
  return errorMessage(error)
}

export function acceptedJobToView(response: PrintResponse, pending: PendingPrint): JobView {
  return {
    id: response.job.job_id,
    printer_id: response.job.printer_id,
    name: null,
    file_id: pending.payload.file_id,
    file_name: response.job.file_name,
    status: response.job.status,
    error: null,
    created_at_ms: response.job.created_at_ms,
    options: pending.payload.options,
  }
}

function sleep(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal.aborted) {
      reject(new DOMException('请求已取消', 'AbortError'))
      return
    }
    const timer = window.setTimeout(() => {
      signal.removeEventListener('abort', onAbort)
      resolve()
    }, ms)
    const onAbort = (): void => {
      window.clearTimeout(timer)
      reject(new DOMException('请求已取消', 'AbortError'))
    }
    signal.addEventListener('abort', onAbort, { once: true })
  })
}

/** 同一幂等键的自动重试：可重试错误指数退避，幂等冲突遵守 Retry-After。 */
export async function submitWithRetry(
  pending: PendingPrint,
  key: string,
  signal: AbortSignal,
): Promise<PrintResponse> {
  let failures = 0
  for (;;) {
    try {
      return await submitPrint(pending.payload, key, signal)
    } catch (error) {
      if (isAbortError(error)) {
        throw error
      }
      const conflict = isIdempotencyConflict(error)
      if ((!isRetryable(error) && !conflict) || failures >= MAX_SUBMIT_RETRIES) {
        throw error
      }
      failures += 1
      const retryAfter = error instanceof ApiError ? error.retryAfterMs : null
      const delay = conflict
        ? (retryAfter ?? 1000)
        : Math.min(1000 * 2 ** (failures - 1), MAX_BACKOFF_MS)
      await sleep(delay, signal)
    }
  }
}

export interface PrintSubmission {
  submit: (pending: PendingPrint) => Promise<void>
  cancel: () => void
}

export function usePrintSubmission(): PrintSubmission {
  const state = useAppState()
  const dispatch = useAppDispatch()
  const authGuard = useAuthGuard()
  const attemptRef = useRef(state.print.attempt)
  attemptRef.current = state.print.attempt
  const controllerRef = useRef<AbortController | null>(null)

  const submit = useCallback(
    async (pending: PendingPrint): Promise<void> => {
      const fingerprint = printFingerprint(pending.payload)
      const { attempt, reused } = resolveAttempt(attemptRef.current, fingerprint)
      dispatch({ type: 'print/begin', attempt })

      const controller = new AbortController()
      controllerRef.current?.abort()
      controllerRef.current = controller

      try {
        const response = await submitWithRetry(pending, attempt.key, controller.signal)
        dispatch({ type: 'jobs/upsert', jobs: [acceptedJobToView(response, pending)] })
        dispatch({ type: 'print/settled' })
        dispatch({
          type: 'notice/add',
          notice: createNotice(
            'success',
            response.replayed
              ? '该打印请求已提交过，已复用同一个 CUPS 任务。'
              : reused
                ? '打印任务已重新提交给 CUPS。'
                : '打印任务已提交给 CUPS。',
          ),
        })
      } catch (error) {
        if (isAbortError(error)) {
          return
        }
        if (authGuard(error)) {
          return
        }
        const message = friendlyPrintError(error)
        dispatch({
          type: 'print/error',
          message,
          retryable: isRetryable(error) || isIdempotencyConflict(error),
          retryAfterMs: error instanceof ApiError ? error.retryAfterMs : null,
        })
        dispatch({ type: 'notice/add', notice: createNotice('error', `提交打印失败：${message}`) })
      } finally {
        if (controllerRef.current === controller) {
          controllerRef.current = null
        }
      }
    },
    [attemptRef, authGuard, dispatch],
  )

  const cancel = useCallback((): void => {
    controllerRef.current?.abort()
    controllerRef.current = null
  }, [])

  return { submit, cancel }
}
