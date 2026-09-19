/* =============================================================================
 * 全局状态：唯一数据源（reducer + context）。
 *
 * 之前 App / Uploader / PrinterPanel / JobList 各自维护 upload、jobs、finishedIds、
 * views 等副本，容易出现不一致。这里把所有可变状态收敛到一个 reducer：
 *   - 组件只读取派生值（步骤、计数、摘要），不再各自维护镜像状态；
 *   - 任务视图按 id 合并并裁剪；文件/预览不会因为任务消失而被清空；
 *   - 打印幂等键作为打印状态的一部分，跨重试复用。
 * ========================================================================== */

import { createContext, createElement } from 'preact'
import type { ComponentChildren } from 'preact'
import { useContext, useReducer } from 'preact/hooks'
import type { Formats, JobStatus, JobView, Printer, PrintPayload, UploadResult } from './api'
import { getStoredToken } from './api'
import { defaultCreateKey, type PrintAttempt } from './idempotency'
import { deriveDefaults } from './options'

/** 任务视图上限，防止长时间轮询导致内存增长。 */
export const MAX_JOB_VIEWS = 50
export const MAX_FINISHED_JOB_VIEWS = 30
export const MAX_JOB_AGE_MS = 24 * 60 * 60 * 1000

/* -------------------------------------------------------------------------- */
/* 通知                                                                        */
/* -------------------------------------------------------------------------- */

export type NoticeKind = 'success' | 'error' | 'info'

export interface Notice {
  id: string
  kind: NoticeKind
  message: string
}

export function createNotice(kind: NoticeKind, message: string): Notice {
  return { id: defaultCreateKey(), kind, message }
}

/* -------------------------------------------------------------------------- */
/* 上传                                                                        */
/* -------------------------------------------------------------------------- */

export type UploadPhase = 'empty' | 'selected' | 'uploading' | 'converting' | 'ready' | 'error'
export type PreviewStatus = 'idle' | 'loading' | 'ready' | 'error'

export interface UploadState {
  phase: UploadPhase
  file: File | null
  /** 0–100，100 表示请求体已发完、正在服务端转换。 */
  progress: number
  result: UploadResult | null
  previewUrl: string | null
  previewStatus: PreviewStatus
  previewError: string | null
  error: string | null
}

function emptyUpload(): UploadState {
  return {
    phase: 'empty',
    file: null,
    progress: 0,
    result: null,
    previewUrl: null,
    previewStatus: 'idle',
    previewError: null,
    error: null,
  }
}

/* -------------------------------------------------------------------------- */
/* 打印机 / 打印                                                               */
/* -------------------------------------------------------------------------- */

export interface PrintersState {
  items: Printer[]
  status: 'idle' | 'loading' | 'ready' | 'error'
  refreshing: boolean
  error: string | null
}

export interface JobsState {
  views: JobView[]
  polling: boolean
  lastError: string | null
  lastUpdatedMs: number | null
}

export interface PendingPrint {
  payload: PrintPayload
  printerName: string
  fileName: string
  summary: { label: string; value: string }[]
}

export interface PrintState {
  submitting: boolean
  error: string | null
  retryable: boolean
  retryAfterMs: number | null
  attempt: PrintAttempt | null
  confirmation: PendingPrint | null
}

export interface AppState {
  token: string
  /** 因 401 被强制退出登录时的说明文案。 */
  authMessage: string | null
  notices: Notice[]
  upload: UploadState
  formats: Formats | null
  formatsError: string | null
  printers: PrintersState
  selectedPrinterId: string
  options: Record<string, string>
  jobs: JobsState
  print: PrintState
}

export function createInitialState(token: string = getStoredToken()): AppState {
  return {
    token,
    authMessage: null,
    notices: [],
    upload: emptyUpload(),
    formats: null,
    formatsError: null,
    printers: { items: [], status: 'idle', refreshing: false, error: null },
    selectedPrinterId: '',
    options: {},
    jobs: { views: [], polling: false, lastError: null, lastUpdatedMs: null },
    print: {
      submitting: false,
      error: null,
      retryable: false,
      retryAfterMs: null,
      attempt: null,
      confirmation: null,
    },
  }
}

/* -------------------------------------------------------------------------- */
/* Actions                                                                     */
/* -------------------------------------------------------------------------- */

export type Action =
  | { type: 'auth/validated'; token: string }
  | { type: 'auth/invalid'; message: string }
  | { type: 'auth/logout' }
  | { type: 'notice/add'; notice: Notice }
  | { type: 'notice/dismiss'; id: string }
  | { type: 'notice/clear' }
  | { type: 'formats/loaded'; formats: Formats }
  | { type: 'formats/error'; message: string }
  | { type: 'upload/select'; file: File | null }
  | { type: 'upload/start' }
  | { type: 'upload/progress'; percent: number }
  | { type: 'upload/converting' }
  | { type: 'upload/success'; result: UploadResult }
  | { type: 'upload/error'; message: string }
  | { type: 'upload/reset' }
  | { type: 'upload/preview-loading' }
  | { type: 'upload/preview-ready'; url: string }
  | { type: 'upload/preview-error'; message: string }
  | { type: 'printers/loading' }
  | { type: 'printers/loaded'; printers: Printer[] }
  | { type: 'printers/refreshing'; value: boolean }
  | { type: 'printers/error'; message: string }
  | { type: 'printer/select'; id: string }
  | { type: 'options/set'; key: string; value: string }
  | { type: 'options/reset' }
  | { type: 'print/confirm'; pending: PendingPrint }
  | { type: 'print/cancel-confirm' }
  | { type: 'print/begin'; attempt: PrintAttempt }
  | { type: 'print/abort' }
  | { type: 'print/reset-attempt' }
  | { type: 'print/settled' }
  | { type: 'print/error'; message: string; retryable: boolean; retryAfterMs: number | null }
  | { type: 'jobs/upsert'; jobs: JobView[] }
  | { type: 'jobs/clear-finished' }
  | { type: 'jobs/polling'; value: boolean }
  | { type: 'jobs/error'; message: string | null }

/* -------------------------------------------------------------------------- */
/* 任务派生                                                                    */
/* -------------------------------------------------------------------------- */

export function isActiveJobStatus(status: JobStatus | null): boolean {
  return status === 'queued' || status === 'printing'
}

export function isTerminalJobStatus(status: JobStatus | null): boolean {
  return status === null || status === 'completed' || status === 'failed' || status === 'canceled'
}

/** 合并任务视图（后来的覆盖先前的），并按上限与时效裁剪。 */
export function mergeJobs(existing: readonly JobView[], incoming: readonly JobView[]): JobView[] {
  const byId = new Map<string, JobView>()
  for (const job of existing) {
    byId.set(job.id, job)
  }
  for (const job of incoming) {
    const previous = byId.get(job.id)
    byId.set(job.id, previous ? { ...previous, ...job } : job)
  }
  return pruneJobs([...byId.values()])
}

export function pruneJobs(views: readonly JobView[], now: number = Date.now()): JobView[] {
  const sorted = [...views].sort((a, b) => b.created_at_ms - a.created_at_ms)
  const active: JobView[] = []
  const finished: JobView[] = []
  for (const job of sorted) {
    if (isActiveJobStatus(job.status)) {
      active.push(job)
      continue
    }
    if (now - job.created_at_ms > MAX_JOB_AGE_MS) {
      continue
    }
    finished.push(job)
  }
  return [
    ...active,
    ...finished.slice(
      0,
      Math.max(0, Math.min(MAX_FINISHED_JOB_VIEWS, MAX_JOB_VIEWS - active.length)),
    ),
  ]
}

export interface JobCounts {
  total: number
  active: number
  completed: number
  failed: number
  canceled: number
  unknown: number
}

export function deriveJobCounts(views: readonly JobView[]): JobCounts {
  const counts: JobCounts = {
    total: views.length,
    active: 0,
    completed: 0,
    failed: 0,
    canceled: 0,
    unknown: 0,
  }
  for (const job of views) {
    if (isActiveJobStatus(job.status)) {
      counts.active += 1
    } else if (job.status === 'completed') {
      counts.completed += 1
    } else if (job.status === 'failed') {
      counts.failed += 1
    } else if (job.status === 'canceled') {
      counts.canceled += 1
    } else {
      counts.unknown += 1
    }
  }
  return counts
}

export type StepState = 'done' | 'active' | 'todo'

/** 上传 → 打印 → 任务状态 的步骤状态，完全由全局状态派生。 */
export function deriveSteps(state: AppState): StepState[] {
  const uploadDone = state.upload.result !== null
  const hasJobs = state.jobs.views.length > 0
  const hasActiveJob = state.jobs.views.some((job) => isActiveJobStatus(job.status))

  const first: StepState = uploadDone ? 'done' : 'active'
  const second: StepState = hasJobs ? 'done' : uploadDone ? 'active' : 'todo'
  const third: StepState = !hasJobs ? 'todo' : hasActiveJob ? 'active' : 'done'
  return [first, second, third]
}

/* -------------------------------------------------------------------------- */
/* Reducer                                                                     */
/* -------------------------------------------------------------------------- */

function resetSession(token: string, authMessage: string | null): AppState {
  return {
    ...createInitialState(token),
    authMessage,
  }
}

export function appReducer(state: AppState, action: Action): AppState {
  switch (action.type) {
    case 'auth/validated':
      return { ...state, token: action.token, authMessage: null }

    case 'auth/invalid':
      return resetSession('', action.message)

    case 'auth/logout':
      return resetSession('', null)

    case 'notice/add':
      return { ...state, notices: [...state.notices, action.notice] }

    case 'notice/dismiss':
      return { ...state, notices: state.notices.filter((notice) => notice.id !== action.id) }

    case 'notice/clear':
      return { ...state, notices: [] }

    case 'formats/loaded':
      return { ...state, formats: action.formats, formatsError: null }

    case 'formats/error':
      return { ...state, formatsError: action.message }

    case 'upload/select':
      return {
        ...state,
        upload: {
          ...emptyUpload(),
          file: action.file,
          phase: action.file ? 'selected' : 'empty',
        },
        print: { ...state.print, error: null, retryable: false, retryAfterMs: null },
      }

    case 'upload/start':
      return {
        ...state,
        upload: {
          ...state.upload,
          phase: 'uploading',
          progress: 0,
          error: null,
          result: null,
          previewUrl: null,
          previewStatus: 'idle',
          previewError: null,
        },
      }

    case 'upload/progress':
      return {
        ...state,
        upload: {
          ...state.upload,
          phase: action.percent >= 100 ? 'converting' : 'uploading',
          progress: action.percent,
        },
      }

    case 'upload/converting':
      return { ...state, upload: { ...state.upload, phase: 'converting', progress: 100 } }

    case 'upload/success':
      return {
        ...state,
        upload: {
          ...state.upload,
          phase: 'ready',
          progress: 100,
          result: action.result,
          error: null,
          previewStatus: 'loading',
          previewError: null,
        },
      }

    case 'upload/error':
      return { ...state, upload: { ...state.upload, phase: 'error', error: action.message } }

    case 'upload/reset':
      return { ...state, upload: emptyUpload() }

    case 'upload/preview-loading':
      return { ...state, upload: { ...state.upload, previewStatus: 'loading', previewError: null } }

    case 'upload/preview-ready':
      return {
        ...state,
        upload: {
          ...state.upload,
          previewUrl: action.url,
          previewStatus: 'ready',
          previewError: null,
        },
      }

    case 'upload/preview-error':
      return {
        ...state,
        upload: { ...state.upload, previewStatus: 'error', previewError: action.message },
      }

    case 'printers/loading':
      return { ...state, printers: { ...state.printers, status: 'loading' } }

    case 'printers/loaded': {
      const { printers } = action
      const currentStillPresent = printers.some((printer) => printer.id === state.selectedPrinterId)
      const selectedPrinterId = currentStillPresent
        ? state.selectedPrinterId
        : (printers[0]?.id ?? '')
      const selected = printers.find((printer) => printer.id === selectedPrinterId) ?? null
      return {
        ...state,
        printers: {
          items: printers,
          status: 'ready',
          refreshing: false,
          error: null,
        },
        selectedPrinterId,
        options: currentStillPresent ? state.options : selected ? deriveDefaults(selected) : {},
      }
    }

    case 'printers/refreshing':
      return { ...state, printers: { ...state.printers, refreshing: action.value } }

    case 'printers/error':
      return {
        ...state,
        printers: { ...state.printers, status: 'error', refreshing: false, error: action.message },
      }

    case 'printer/select': {
      const printer = state.printers.items.find((candidate) => candidate.id === action.id)
      return {
        ...state,
        selectedPrinterId: action.id,
        options: printer ? deriveDefaults(printer) : {},
        print: { ...state.print, error: null, retryable: false, retryAfterMs: null },
      }
    }

    case 'options/set':
      return {
        ...state,
        options: { ...state.options, [action.key]: action.value },
        print: { ...state.print, error: null, retryable: false, retryAfterMs: null },
      }

    case 'options/reset': {
      const printer = state.printers.items.find(
        (candidate) => candidate.id === state.selectedPrinterId,
      )
      return { ...state, options: printer ? deriveDefaults(printer) : {} }
    }

    case 'print/confirm':
      return {
        ...state,
        print: {
          ...state.print,
          confirmation: action.pending,
          error: null,
          retryable: false,
          retryAfterMs: null,
        },
      }

    case 'print/cancel-confirm':
      return { ...state, print: { ...state.print, confirmation: null } }

    case 'print/begin':
      return {
        ...state,
        print: {
          ...state.print,
          submitting: true,
          error: null,
          retryable: false,
          retryAfterMs: null,
          attempt: action.attempt,
        },
      }

    case 'print/abort':
      // 提交被取消（例如退出登录或用户中止）：必须清掉 submitting，
      // 否则确认对话框会永远停在 busy（全部按钮禁用、Esc 也失效）。
      return {
        ...state,
        print: {
          ...state.print,
          submitting: false,
          confirmation: null,
          error: null,
          retryable: false,
          retryAfterMs: null,
        },
      }

    case 'print/reset-attempt':
      return {
        ...state,
        print: { ...state.print, attempt: null, error: null, retryable: false, retryAfterMs: null },
      }

    case 'print/settled':
      return {
        ...state,
        print: {
          ...state.print,
          submitting: false,
          error: null,
          retryable: false,
          retryAfterMs: null,
          confirmation: null,
          attempt: state.print.attempt ? { ...state.print.attempt, settled: true } : null,
        },
      }

    case 'print/error':
      return {
        ...state,
        print: {
          ...state.print,
          submitting: false,
          error: action.message,
          retryable: action.retryable,
          retryAfterMs: action.retryAfterMs,
        },
      }

    case 'jobs/upsert':
      return {
        ...state,
        jobs: {
          ...state.jobs,
          views: mergeJobs(state.jobs.views, action.jobs),
          lastError: null,
          lastUpdatedMs: Date.now(),
        },
      }

    case 'jobs/clear-finished':
      return {
        ...state,
        jobs: {
          ...state.jobs,
          views: state.jobs.views.filter((job) => isActiveJobStatus(job.status)),
        },
      }

    case 'jobs/polling':
      return { ...state, jobs: { ...state.jobs, polling: action.value } }

    case 'jobs/error':
      return { ...state, jobs: { ...state.jobs, lastError: action.message } }
  }
}

/* -------------------------------------------------------------------------- */
/* Context                                                                     */
/* -------------------------------------------------------------------------- */

export interface AppContextValue {
  state: AppState
  dispatch: (action: Action) => void
}

const AppStateContext = createContext<AppContextValue | null>(null)

export function AppStateProvider({ children }: { children: ComponentChildren }) {
  const [state, dispatch] = useReducer(appReducer, undefined, () => createInitialState())
  return createElement(AppStateContext.Provider, { value: { state, dispatch } }, children)
}

export function useAppState(): AppState {
  const context = useContext(AppStateContext)
  if (context === null) {
    throw new Error('useAppState 必须在 AppStateProvider 内使用')
  }
  return context.state
}

export function useAppDispatch(): (action: Action) => void {
  const context = useContext(AppStateContext)
  if (context === null) {
    throw new Error('useAppDispatch 必须在 AppStateProvider 内使用')
  }
  return context.dispatch
}
