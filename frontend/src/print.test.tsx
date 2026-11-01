import type { ComponentChildren } from 'preact'
import { act, renderHook } from '@testing-library/preact'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { clearStoredToken } from './api'
import { usePrintSubmission } from './print'
import { AppStateProvider, useAppDispatch, useAppState, type PendingPrint } from './state'

const pending: PendingPrint = {
  payload: { file_id: 'file-1', printer_id: 'CUPS-PDF', options: {} },
  printerName: 'CUPS-PDF',
  fileName: 'report.pdf',
  summary: [],
}

function wrapper({ children }: { children?: ComponentChildren }) {
  return <AppStateProvider>{children}</AppStateProvider>
}

function accepted(): Response {
  return new Response(
    JSON.stringify({
      job_id: 'CUPS-PDF-8',
      printer_id: 'CUPS-PDF',
      status: 'queued',
      created_at_ms: 1,
      file_name: 'report.pdf',
    }),
    { status: 202, headers: { 'Content-Type': 'application/json' } },
  )
}

describe('打印提交的会话隔离', () => {
  beforeEach(() => {
    sessionStorage.setItem('just_print_token', 'old-token')
  })

  afterEach(() => {
    vi.unstubAllGlobals()
    sessionStorage.clear()
  })

  it('登出后迟到的成功响应不会写回任务或通知', async () => {
    let resolveFetch: ((response: Response) => void) | null = null
    vi.stubGlobal(
      'fetch',
      vi.fn(
        () =>
          new Promise<Response>((resolve) => {
            resolveFetch = resolve
          }),
      ),
    )

    const hook = renderHook(
      () => ({
        submission: usePrintSubmission(),
        dispatch: useAppDispatch(),
        state: useAppState(),
      }),
      { wrapper },
    )

    await act(async () => {
      const submission = hook.result.current.submission.submit(pending)
      // 请求发出后、响应回来前退出登录并递增会话代次。
      clearStoredToken()
      hook.result.current.dispatch({ type: 'auth/logout' })
      resolveFetch?.(accepted())
      await submission
    })

    expect(hook.result.current.state.token).toBe('')
    expect(hook.result.current.state.jobs.views).toEqual([])
    expect(hook.result.current.state.notices).toEqual([])
    expect(hook.result.current.state.print.submitting).toBe(false)
  })
})
