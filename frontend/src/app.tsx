import { useEffect, useState } from 'preact/hooks'
import {
  type UploadResult,
  clearStoredToken,
  getStoredToken,
} from './api'
import { TokenGate } from './components/TokenGate'
import { Uploader } from './components/Uploader'
import { PrinterPanel } from './components/PrinterPanel'
import { JobList, type JobEntry } from './components/JobList'
import { FlowSteps, type StepState } from './components/FlowSteps'
import {
  AlertIcon,
  CheckIcon,
  GitHubIcon,
  InfoIcon,
  LogoMark,
  LogoutIcon,
} from './icons'
import './app.css'

export type NoticeKind = 'success' | 'error' | 'info'

export interface Notice {
  message: string
  kind: NoticeKind
}

export function App() {
  const [token, setToken] = useState<string>(() => getStoredToken())
  const [upload, setUpload] = useState<UploadResult | null>(null)
  const [jobs, setJobs] = useState<JobEntry[]>([])
  const [finishedIds, setFinishedIds] = useState<Set<string>>(new Set())
  const [notice, setNotice] = useState<Notice | null>(null)

  useEffect(() => {
    if (!notice) {
      return
    }
    const timer = window.setTimeout(() => setNotice(null), 6000)
    return () => window.clearTimeout(timer)
  }, [notice])

  function handleAuthFailure(): void {
    clearStoredToken()
    setToken('')
    setUpload(null)
    setJobs([])
    setFinishedIds(new Set())
    setNotice(null)
  }

  function handleUploaded(result: UploadResult): void {
    setUpload(result)
    setNotice({ message: `文件「${result.name}」已转换完成`, kind: 'success' })
  }

  function handleUploadInvalid(): void {
    setUpload(null)
    setNotice({ message: '服务已重启，请重新上传', kind: 'error' })
  }

  function handleFinished(ids: string[]): void {
    setFinishedIds((previous) => {
      const next = new Set(previous)
      for (const id of ids) {
        next.add(id)
      }
      return next
    })
  }

  function handleClearFinished(): void {
    setJobs((previous) => previous.filter((job) => !finishedIds.has(job.id)))
    setFinishedIds(new Set())
  }

  if (!token) {
    return <TokenGate onValid={(value) => setToken(value)} />
  }

  const allFinished = jobs.length > 0 && jobs.every((job) => finishedIds.has(job.id))
  const stepStates: StepState[] = [
    upload ? 'done' : 'active',
    jobs.length > 0 ? 'done' : upload ? 'active' : 'todo',
    jobs.length > 0 ? (allFinished ? 'done' : 'active') : 'todo',
  ]

  return (
    <div class="app-shell">
      <header class="app-header">
        <div class="brand">
          <span class="brand-logo">
            <LogoMark size={42} />
          </span>
          <div class="brand-text">
            <h1>Just Print</h1>
            <span class="tagline">CUPS 驱动 · 文档打印服务</span>
          </div>
        </div>
        <div class="header-actions">
          <a
            class="ghost github-link"
            href="https://github.com/NIyueeE/just-print"
            target="_blank"
            rel="noreferrer"
            aria-label="Just Print GitHub 仓库"
            title="Just Print GitHub 仓库"
          >
            <GitHubIcon size={18} />
          </a>
          <button type="button" class="ghost" onClick={handleAuthFailure}>
            <LogoutIcon size={16} />
            退出登录
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
        <Uploader
          onUploaded={handleUploaded}
          onAuthFailure={handleAuthFailure}
          onUploadInvalid={handleUploadInvalid}
          onNotice={setNotice}
        />
        <PrinterPanel
          upload={upload}
          onJobSubmitted={(jobId, printerName) =>
            setJobs((previous) => [
              { id: jobId, name: upload?.name ?? '文档', printer: printerName },
              ...previous,
            ])}
          onAuthFailure={handleAuthFailure}
          onNotice={setNotice}
        />
        <JobList
          jobs={jobs}
          onAuthFailure={handleAuthFailure}
          onRestart={handleUploadInvalid}
          onFinished={handleFinished}
          onClearFinished={handleClearFinished}
        />
      </main>
      {notice ? (
        <div class={`notice notice-${notice.kind}`} role="status">
          {notice.kind === 'success' ? (
            <CheckIcon size={16} />
          ) : notice.kind === 'error' ? (
            <AlertIcon size={16} />
          ) : (
            <InfoIcon size={16} />
          )}
          {notice.message}
        </div>
      ) : null}
    </div>
  )
}
