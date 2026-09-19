import { useEffect, useRef } from 'preact/hooks'
import {
  ApiError,
  cancelJob,
  errorMessage,
  getJob,
  isAbortError,
  isUnauthorized,
  listJobs,
  retryAfterHint,
  type JobStatus,
  type JobView,
} from '../api'
import { formatAbsoluteTime, formatRelativeTime } from '../format'
import { useAuthGuard } from '../hooks/useAuthGuard'
import { usePolling } from '../hooks/usePolling'
import {
  AlertIcon,
  BanIcon,
  CheckIcon,
  ClockIcon,
  InfoIcon,
  PrinterIcon,
  RedoIcon,
  SpinnerIcon,
  TrashIcon,
} from '../icons'
import { summarizeOptions } from '../options'
import {
  createNotice,
  deriveJobCounts,
  isActiveJobStatus,
  MAX_JOB_VIEWS,
  useAppDispatch,
  useAppState,
  type PendingPrint,
} from '../state'
import './JobList.css'

type DisplayStatus = JobStatus | 'unknown'

const STATUS_LABEL: Record<DisplayStatus, string> = {
  queued: '排队中',
  printing: '打印中',
  completed: '已完成',
  failed: '失败',
  canceled: '已取消',
  unknown: '已清理',
}

function displayStatus(job: JobView): DisplayStatus {
  return job.status ?? 'unknown'
}

function JobStatusIcon({ status }: { status: DisplayStatus }) {
  if (status === 'queued') return <ClockIcon size={16} />
  if (status === 'printing') return <SpinnerIcon size={16} />
  if (status === 'completed') return <CheckIcon size={16} />
  return <AlertIcon size={16} />
}

export function JobList() {
  const state = useAppState()
  const dispatch = useAppDispatch()
  const authGuard = useAuthGuard()
  const { jobs, printers } = state

  const viewsRef = useRef(jobs.views)
  viewsRef.current = jobs.views
  const hydratedRef = useRef(false)

  // 首次挂载（含刷新页面后）拉取一次任务列表，恢复本地视图与计数。
  useEffect(() => {
    if (hydratedRef.current || state.token === '') {
      return
    }
    hydratedRef.current = true
    const controller = new AbortController()
    listJobs({ limit: MAX_JOB_VIEWS, signal: controller.signal }).then(
      (loaded) => dispatch({ type: 'jobs/upsert', jobs: loaded }),
      (error: unknown) => {
        if (isAbortError(error) || controller.signal.aborted) {
          return
        }
        authGuard(error)
      },
    )
    return () => controller.abort()
  }, [state.token, dispatch, authGuard])

  const hasActive = jobs.views.some((job) => isActiveJobStatus(job.status))
  const counts = deriveJobCounts(jobs.views)
  const terminalCount = counts.completed + counts.failed + counts.canceled + counts.unknown

  // 3 秒轮询进行中的任务；请求不重叠（usePolling 内部守卫）、可中断、失败退避。
  usePolling(
    async (signal) => {
      dispatch({ type: 'jobs/polling', value: true })
      try {
        const activeJobs = await listJobs({ active: true, limit: MAX_JOB_VIEWS, signal })
        dispatch({ type: 'jobs/upsert', jobs: activeJobs })

        // 本地认为还在进行、但已不在 active 列表里的任务：逐个查询终态。
        const activeIds = new Set(activeJobs.map((job) => job.id))
        const pending = viewsRef.current.filter(
          (job) => isActiveJobStatus(job.status) && !activeIds.has(job.id),
        )
        for (const job of pending) {
          if (signal.aborted) {
            return
          }
          try {
            const fresh = await getJob(job.id, signal)
            dispatch({ type: 'jobs/upsert', jobs: [fresh] })
          } catch (error) {
            if (isAbortError(error)) {
              return
            }
            if (isUnauthorized(error)) {
              throw error
            }
            if (error instanceof ApiError && error.status === 404) {
              // CUPS 已清理该任务：只更新任务视图，绝不清空已上传的文件/预览。
              dispatch({ type: 'jobs/upsert', jobs: [{ ...job, status: null }] })
            }
          }
        }
      } catch (error) {
        if (isAbortError(error) || signal.aborted) {
          return
        }
        if (authGuard(error)) {
          return
        }
        dispatch({
          type: 'jobs/error',
          message: `任务状态刷新失败：${errorMessage(error)}${retryAfterHint(error)}`,
        })
        throw error
      } finally {
        dispatch({ type: 'jobs/polling', value: false })
      }
    },
    { intervalMs: 3000, enabled: hasActive, backoffBaseMs: 2000, backoffMaxMs: 30_000 },
  )

  if (jobs.views.length === 0) {
    return null
  }

  async function handleCancel(job: JobView): Promise<void> {
    try {
      await cancelJob(job.id)
      dispatch({
        type: 'notice/add',
        notice: createNotice('success', `已请求取消任务 ${job.id}，状态稍后刷新。`),
      })
    } catch (error) {
      if (authGuard(error)) {
        return
      }
      dispatch({
        type: 'notice/add',
        notice: createNotice('error', `取消任务失败：${errorMessage(error)}`),
      })
    }
  }

  function handleReprint(job: JobView): void {
    if (job.file_id === null) {
      return
    }
    const printer = printers.items.find((candidate) => candidate.id === job.printer_id) ?? null
    const summary =
      printer !== null
        ? summarizeOptions(printer, job.options).map((row) => ({
            label: row.label,
            value: row.value,
          }))
        : Object.entries(job.options).map(([label, value]) => ({ label, value }))
    const pending: PendingPrint = {
      payload: {
        file_id: job.file_id,
        printer_id: job.printer_id,
        options: job.options,
      },
      printerName: printer?.name ?? job.printer_id,
      fileName: job.file_name ?? job.name ?? '文档',
      summary,
    }
    // 重新打印是新的逻辑尝试：先清空旧幂等键，再走确认对话框。
    dispatch({ type: 'print/reset-attempt' })
    dispatch({ type: 'print/confirm', pending })
  }

  return (
    <section class="card jobs" aria-labelledby="jobs-title">
      <div class="card__header">
        <span class="step-badge step-badge--jobs" aria-hidden="true">
          3
        </span>
        <span class="card__icon card__icon--jobs" aria-hidden="true">
          <ClockIcon size={18} />
        </span>
        <h2 id="jobs-title">任务状态</h2>
        {terminalCount > 0 ? (
          <button
            type="button"
            class="ghost ghost--sm jobs__clear"
            onClick={() => dispatch({ type: 'jobs/clear-finished' })}
            title="从列表移除已结束的任务"
          >
            <TrashIcon size={14} />
            清除已结束（{terminalCount}）
          </button>
        ) : null}
      </div>

      <p class="jobs__counts" aria-live="polite">
        共 {counts.total} 个 · 进行中 {counts.active} · 已完成 {counts.completed} · 失败{' '}
        {counts.failed} · 已取消 {counts.canceled}
        {counts.unknown > 0 ? ` · 已清理 ${counts.unknown}` : ''}
        {jobs.polling ? ' · 刷新中…' : ''}
      </p>

      {jobs.lastError !== null ? (
        <div class="inline-error" role="alert">
          <AlertIcon size={16} />
          <span>{jobs.lastError}</span>
        </div>
      ) : null}

      <ul class="jobs__list" aria-live="polite" aria-busy={jobs.polling}>
        {jobs.views.map((job) => {
          const status = displayStatus(job)
          const printerName =
            printers.items.find((printer) => printer.id === job.printer_id)?.name ?? job.printer_id
          const name = job.file_name ?? job.name ?? job.id
          const active = isActiveJobStatus(job.status)
          return (
            <li key={job.id} class={`job job--${status}`}>
              <span class={`job__icon job__icon--${status}`} aria-hidden="true">
                <JobStatusIcon status={status} />
              </span>
              <div class="job__main">
                <div class="job__topline">
                  <span class="job__name" title={name}>
                    {name}
                  </span>
                  <span class={`badge badge--${status}`}>{STATUS_LABEL[status]}</span>
                </div>
                <div class="job__subline">
                  <span class="job__printer" title={printerName}>
                    <PrinterIcon size={12} />
                    {printerName}
                  </span>
                  <span class="job__id mono">{job.id}</span>
                  <span class="job__time" title={formatAbsoluteTime(job.created_at_ms)}>
                    {formatRelativeTime(job.created_at_ms)}
                  </span>
                </div>
                {status === 'printing' ? <span class="job__progress" aria-hidden="true" /> : null}
                {status === 'failed' && job.error !== null ? (
                  <span class="job__error">
                    <AlertIcon size={13} />
                    {job.error}
                  </span>
                ) : null}
                {job.file_id === null && !active ? (
                  <span class="job__note">
                    <InfoIcon size={12} />
                    服务重启后任务元数据已丢失，无法重新打印。
                  </span>
                ) : null}
                {active ||
                (job.file_id !== null && (status === 'failed' || status === 'canceled')) ? (
                  <div class="job__actions">
                    {active ? (
                      <button
                        type="button"
                        class="ghost ghost--sm"
                        onClick={() => void handleCancel(job)}
                      >
                        <BanIcon size={14} />
                        取消任务
                      </button>
                    ) : null}
                    {job.file_id !== null && (status === 'failed' || status === 'canceled') ? (
                      <button
                        type="button"
                        class="ghost ghost--sm"
                        onClick={() => handleReprint(job)}
                      >
                        <RedoIcon size={14} />
                        重新打印
                      </button>
                    ) : null}
                  </div>
                ) : null}
              </div>
            </li>
          )
        })}
      </ul>
    </section>
  )
}
