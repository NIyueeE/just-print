import { useEffect } from 'preact/hooks'
import { clearStoredToken, errorMessage, getFormats, isAbortError } from './api'
import { ConfirmDialog } from './components/ConfirmDialog'
import { FlowSteps } from './components/FlowSteps'
import { JobList } from './components/JobList'
import { Notices } from './components/Notices'
import { PrinterPanel } from './components/PrinterPanel'
import { ThemeToggle } from './components/ThemeToggle'
import { TokenGate } from './components/TokenGate'
import { Uploader } from './components/Uploader'
import { formatDurationMs } from './format'
import { GitHubIcon, LogoMark, LogoutIcon } from './icons'
import { useAuthGuard } from './hooks/useAuthGuard'
import { optionDoc } from './options'
import { usePrintSubmission } from './print'
import { deriveSteps, useAppDispatch, useAppState } from './state'
import { Tooltip } from './components/Tooltip'
import './app.css'

export function App() {
  const state = useAppState()
  const dispatch = useAppDispatch()
  const authGuard = useAuthGuard()
  const { submit } = usePrintSubmission()
  const { token, authMessage, formats, print } = state

  // 支持格式与大小上限完全来自 /api/formats（不再硬编码扩展名列表）。
  useEffect(() => {
    if (token === '' || formats !== null) {
      return
    }
    const controller = new AbortController()
    getFormats(controller.signal).then(
      (loaded) => dispatch({ type: 'formats/loaded', formats: loaded }),
      (error: unknown) => {
        if (isAbortError(error) || controller.signal.aborted) {
          return
        }
        if (authGuard(error)) {
          return
        }
        dispatch({ type: 'formats/error', message: errorMessage(error) })
      },
    )
    return () => controller.abort()
  }, [token, formats, dispatch, authGuard])

  if (token === '') {
    return (
      <TokenGate
        message={authMessage}
        onValid={(value) => dispatch({ type: 'auth/validated', token: value })}
      />
    )
  }

  const stepStates = deriveSteps(state)
  const confirmation = print.confirmation
  // 服务端要求等待时把等待时长写进错误文案，否则用户会立刻重试、再撞一次同样的错误。
  const printError =
    print.error === null
      ? null
      : print.retryAfterMs !== null && print.retryAfterMs > 0
        ? `${print.error}（服务端建议约 ${formatDurationMs(print.retryAfterMs)}后重试）`
        : print.error

  function handleLogout(): void {
    clearStoredToken()
    dispatch({ type: 'auth/logout' })
  }

  return (
    <div class="app-shell">
      <header class="app-header">
        <div class="brand">
          <span class="brand__logo" aria-hidden="true">
            <LogoMark size={42} />
          </span>
          <div class="brand__text">
            <h1>Just Print</h1>
            <span class="tagline">CUPS 驱动 · 文档打印服务</span>
          </div>
        </div>
        <div class="header-actions">
          <ThemeToggle />
          <a
            class="ghost ghost--icon github-link"
            href="https://github.com/NIyueeE/just-print"
            target="_blank"
            rel="noreferrer"
            aria-label="Just Print GitHub 仓库"
            title="Just Print GitHub 仓库"
          >
            <GitHubIcon size={18} />
          </a>
          <button
            type="button"
            class="ghost ghost--icon tip"
            data-tip="退出登录"
            aria-label="退出登录"
            onClick={handleLogout}
          >
            <LogoutIcon size={16} />
          </button>
        </div>
      </header>

      <FlowSteps
        steps={[
          { label: '上传文档', state: stepStates[0] },
          { label: '打印设置', state: stepStates[1] },
          { label: '任务状态', state: stepStates[2] },
        ]}
      />

      <main class="app-main">
        <Uploader />
        <PrinterPanel />
        <JobList />
      </main>

      <ConfirmDialog
        open={confirmation !== null}
        title="确认打印"
        description="打印是物理操作，提交后无法撤回。"
        confirmLabel={print.error !== null ? '重试提交' : '确认打印'}
        busyLabel="提交中…"
        busy={print.submitting}
        error={printError}
        onConfirm={() => {
          if (confirmation !== null) {
            void submit(confirmation)
          }
        }}
        onCancel={() => dispatch({ type: 'print/cancel-confirm' })}
      >
        {confirmation !== null ? (
          <dl class="confirm-summary">
            <div class="confirm-summary__row">
              <dt>文件</dt>
              <dd>{confirmation.fileName}</dd>
            </div>
            <div class="confirm-summary__row">
              <dt>打印机</dt>
              <dd>{confirmation.printerName}</dd>
            </div>
            {confirmation.summary.map((row) => (
              <div class="confirm-summary__row" key={row.label}>
                <dt>
                  {row.key !== null ? (
                    <Tooltip tip={optionDoc(row.key)}>{row.label}</Tooltip>
                  ) : (
                    row.label
                  )}
                </dt>
                <dd>{row.value}</dd>
              </div>
            ))}
          </dl>
        ) : null}
      </ConfirmDialog>

      <Notices />
    </div>
  )
}
