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
import { CheckIcon, GitHubIcon, LogoMark, LogoutIcon } from './icons'
import './app.css'

export function App() {
  const [token, setToken] = useState<string>(() => getStoredToken())
  const [upload, setUpload] = useState<UploadResult | null>(null)
  const [jobs, setJobs] = useState<JobEntry[]>([])
  const [notice, setNotice] = useState<string | null>(null)

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
    setNotice(null)
  }

  function handleUploaded(result: UploadResult): void {
    setUpload(result)
    setNotice(`文件「${result.name}」已转换完成`)
  }

  function handleUploadInvalid(): void {
    setUpload(null)
    setNotice('服务已重启，请重新上传')
  }

  if (!token) {
    return <TokenGate onValid={(value) => setToken(value)} />
  }

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
      <main class="app-main">
        <Uploader
          onUploaded={handleUploaded}
          onAuthFailure={handleAuthFailure}
          onUploadInvalid={handleUploadInvalid}
          onNotice={setNotice}
        />
        <PrinterPanel
          upload={upload}
          onJobSubmitted={(jobId) =>
            setJobs((previous) => [
              { id: jobId, name: upload?.name ?? '文档' },
              ...previous,
            ])}
          onAuthFailure={handleAuthFailure}
          onNotice={setNotice}
        />
        <JobList
          jobs={jobs}
          onAuthFailure={handleAuthFailure}
          onRestart={handleUploadInvalid}
        />
      </main>
      {notice ? (
        <div class="notice" role="status">
          <CheckIcon size={16} />
          {notice}
        </div>
      ) : null}
    </div>
  )
}
