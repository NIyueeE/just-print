import { useEffect, useRef, useState } from 'preact/hooks'
import {
  ApiError,
  type JobStatus,
  getJob,
  isUnauthorized,
} from '../api'
import {
  AlertIcon,
  CheckIcon,
  ClockIcon,
  PrinterIcon,
  SpinnerIcon,
  TrashIcon,
} from '../icons'

export interface JobEntry {
  id: string
  name: string
  printer?: string
}

interface JobListProps {
  jobs: JobEntry[]
  onAuthFailure: () => void
  onRestart: () => void
  onFinished: (ids: string[]) => void
  onClearFinished: () => void
}

interface JobView {
  status: JobStatus
  error: string | null
  createdAtMs: number
}

const STATUS_LABEL: Record<JobStatus, string> = {
  queued: '排队中',
  printing: '打印中',
  completed: '已完成',
  failed: '失败',
  canceled: '已取消',
}

function formatRelative(ms: number): string {
  const elapsed = Date.now() - ms
  if (elapsed < 60_000) {
    return '刚刚提交'
  }
  if (elapsed < 3_600_000) {
    return `${Math.floor(elapsed / 60_000)} 分钟前提交`
  }
  const date = new Date(ms)
  const sameDay = date.toDateString() === new Date().toDateString()
  const time = date.toLocaleTimeString('zh-CN', {
    hour: '2-digit',
    minute: '2-digit',
  })
  return sameDay ? `${time} 提交` : `${date.getMonth() + 1}月${date.getDate()}日 ${time} 提交`
}

function formatAbsolute(ms: number): string {
  return new Date(ms).toLocaleString('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  })
}

export function JobList({
  jobs,
  onAuthFailure,
  onRestart,
  onFinished,
  onClearFinished,
}: JobListProps) {
  const [views, setViews] = useState<Record<string, JobView>>({})
  const finishedRef = useRef<Set<string>>(new Set())
  const handlersRef = useRef({ onAuthFailure, onRestart, onFinished })
  handlersRef.current = { onAuthFailure, onRestart, onFinished }

  useEffect(() => {
    if (jobs.length === 0) {
      return
    }
    let cancelled = false

    async function pollAll(): Promise<void> {
      const newlyFinished: string[] = []
      for (const { id } of jobs) {
        if (finishedRef.current.has(id)) {
          continue
        }
        try {
          const job = await getJob(id)
          if (cancelled) {
            return
          }
          setViews((previous) => ({
            ...previous,
            [id]: {
              status: job.status,
              error: job.error,
              createdAtMs: job.created_at_ms,
            },
          }))
          if (
            job.status === 'completed' ||
            job.status === 'failed' ||
            job.status === 'canceled'
          ) {
            finishedRef.current.add(id)
            newlyFinished.push(id)
          }
        } catch (requestError) {
          if (cancelled) {
            return
          }
          if (isUnauthorized(requestError)) {
            handlersRef.current.onAuthFailure()
            return
          }
          if (requestError instanceof ApiError && requestError.status === 404) {
            finishedRef.current.add(id)
            newlyFinished.push(id)
            setViews((previous) => ({
              ...previous,
              [id]: {
                status: 'failed',
                error: '文件已失效（服务可能已重启）',
                createdAtMs: 0,
              },
            }))
            handlersRef.current.onRestart()
            return
          }
        }
      }
      if (newlyFinished.length > 0) {
        handlersRef.current.onFinished(newlyFinished)
      }
    }

    void pollAll()
    const interval = window.setInterval(() => void pollAll(), 1000)
    return () => {
      cancelled = true
      window.clearInterval(interval)
    }
  }, [jobs])

  if (jobs.length === 0) {
    return null
  }

  const finishedCount = jobs.filter((job) => finishedRef.current.has(job.id)).length

  return (
    <section class="card jobs-section card-jobs">
      <div class="card-header">
        <span class="step-badge">3</span>
        <span class="card-icon">
          <ClockIcon size={18} />
        </span>
        <h2>任务状态</h2>
        {finishedCount > 0 ? (
          <button
            type="button"
            class="ghost ghost-sm clear-jobs"
            onClick={onClearFinished}
            title="移除已结束的任务"
          >
            <TrashIcon size={14} />
            清除已完成{finishedCount > 0 ? `（${finishedCount}）` : ''}
          </button>
        ) : null}
      </div>
      <ul class="jobs">
        {jobs.map(({ id, name, printer }) => {
          const view = views[id]
          const status: JobStatus = view?.status ?? 'queued'
          return (
            <li key={id} class={`job job-${status}`}>
              <span class={`job-icon job-icon-${status}`}>
                {status === 'queued' ? (
                  <ClockIcon size={16} />
                ) : status === 'printing' ? (
                  <SpinnerIcon size={16} />
                ) : status === 'completed' ? (
                  <CheckIcon size={16} />
                ) : (
                  <AlertIcon size={16} />
                )}
              </span>
              <div class="job-main">
                <div class="job-topline">
                  <span class="job-name" title={name}>
                    {name}
                  </span>
                  <span class={`badge badge-${status}`}>{STATUS_LABEL[status]}</span>
                </div>
                <div class="job-subline">
                  {printer ? (
                    <span class="job-printer" title={printer}>
                      <PrinterIcon size={12} />
                      {printer}
                    </span>
                  ) : null}
                  <span class="job-id">{id}</span>
                  <span class="job-time" title={view?.createdAtMs ? formatAbsolute(view.createdAtMs) : undefined}>
                    {view?.createdAtMs
                      ? formatRelative(view.createdAtMs)
                      : '等待状态更新…'}
                  </span>
                </div>
                {status === 'printing' ? <span class="job-progress" /> : null}
                {status === 'failed' && view?.error ? (
                  <span class="job-error">
                    <AlertIcon size={13} />
                    {view.error}
                  </span>
                ) : null}
              </div>
            </li>
          )
        })}
      </ul>
    </section>
  )
}
