import { useEffect, useRef, useState } from 'preact/hooks'
import {
  ApiError,
  type JobStatus,
  getJob,
  isUnauthorized,
} from '../api'

interface JobListProps {
  jobs: string[]
  onAuthFailure: () => void
  onRestart: () => void
}

interface JobView {
  status: JobStatus
  error: string | null
}

const STATUS_LABEL: Record<JobStatus, string> = {
  queued: '排队中',
  printing: '打印中',
  success: '成功',
  failed: '失败',
}

export function JobList({ jobs, onAuthFailure, onRestart }: JobListProps) {
  const [views, setViews] = useState<Record<string, JobView>>({})
  const finishedRef = useRef<Set<string>>(new Set())
  const handlersRef = useRef({ onAuthFailure, onRestart })
  handlersRef.current = { onAuthFailure, onRestart }

  useEffect(() => {
    if (jobs.length === 0) {
      return
    }
    let cancelled = false

    async function pollAll(): Promise<void> {
      for (const id of jobs) {
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
            [id]: { status: job.status, error: job.error },
          }))
          if (job.status === 'success' || job.status === 'failed') {
            finishedRef.current.add(id)
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
            handlersRef.current.onRestart()
            return
          }
        }
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

  return (
    <section class="card jobs-section">
      <h2>任务状态</h2>
      <ul class="jobs">
        {jobs.map((id) => {
          const view = views[id]
          const status: JobStatus = view?.status ?? 'queued'
          return (
            <li key={id} class={`job job-${status}`}>
              <span class="job-id">{id.slice(0, 10)}…</span>
              <span class={`badge badge-${status}`}>{STATUS_LABEL[status]}</span>
              {status === 'failed' && view?.error ? (
                <span class="job-error">{view.error}</span>
              ) : null}
            </li>
          )
        })}
      </ul>
    </section>
  )
}
