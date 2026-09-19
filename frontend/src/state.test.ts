import { describe, expect, it } from 'vitest'
import type { JobView, Printer } from './api'
import { printFingerprint, resolveAttempt } from './idempotency'
import {
  appReducer,
  createInitialState,
  createNotice,
  deriveJobCounts,
  deriveSteps,
  isActiveJobStatus,
  MAX_JOB_AGE_MS,
  pruneJobs,
  type AppState,
} from './state'

function job(overrides: Partial<JobView>): JobView {
  return {
    id: 'job-1',
    printer_id: 'CUPS-PDF',
    name: null,
    file_id: 'file-1',
    file_name: 'report.pdf',
    status: 'queued',
    error: null,
    created_at_ms: Date.now(),
    options: {},
    ...overrides,
  }
}

function printer(id: string, name: string): Printer {
  return {
    id,
    name,
    state: 'idle',
    accepting_jobs: true,
    make_and_model: null,
    location: null,
    options_error: null,
    options: {
      media: {
        kind: 'keyword',
        default: 'na_letter_8.5x11in',
        values: ['na_letter_8.5x11in', 'iso_a4_210x297mm'],
      },
    },
  }
}

function reduce(...actions: Parameters<typeof appReducer>[1][]): AppState {
  return actions.reduce((state, action) => appReducer(state, action), createInitialState('token'))
}

describe('认证与通知', () => {
  it('returns to the gate with an explanatory message on 401', () => {
    const state = reduce({ type: 'auth/invalid', message: '登录状态已失效' })
    expect(state.token).toBe('')
    expect(state.authMessage).toBe('登录状态已失效')
    expect(state.upload.result).toBeNull()
    expect(state.jobs.views).toEqual([])
  })

  it('stacks notices and dismisses them individually', () => {
    const first = createNotice('error', '提交失败')
    const second = createNotice('success', '已提交')
    const state = reduce(
      { type: 'notice/add', notice: first },
      { type: 'notice/add', notice: second },
      { type: 'notice/dismiss', id: first.id },
    )
    expect(state.notices.map((notice) => notice.id)).toEqual([second.id])
  })
})

describe('上传状态机', () => {
  it('goes empty → selected → uploading → converting → ready', () => {
    const file = new File(['x'], 'a.pdf', { type: 'application/pdf' })
    let state = reduce({ type: 'upload/select', file })
    expect(state.upload.phase).toBe('selected')

    state = appReducer(state, { type: 'upload/start' })
    expect(state.upload.phase).toBe('uploading')

    state = appReducer(state, { type: 'upload/progress', percent: 40 })
    expect(state.upload.phase).toBe('uploading')

    state = appReducer(state, { type: 'upload/progress', percent: 100 })
    expect(state.upload.phase).toBe('converting')

    state = appReducer(state, {
      type: 'upload/success',
      result: { id: 'file-1', name: 'a.pdf', size: 1 },
    })
    expect(state.upload.phase).toBe('ready')
    expect(state.upload.previewStatus).toBe('loading')

    state = appReducer(state, { type: 'upload/preview-ready', url: 'blob:preview' })
    expect(state.upload.previewStatus).toBe('ready')
    expect(state.upload.previewUrl).toBe('blob:preview')
  })

  it('keeps the upload when a preview fails (no false “service restarted”)', () => {
    let state = reduce(
      { type: 'upload/select', file: new File(['x'], 'a.pdf') },
      { type: 'upload/success', result: { id: 'file-1', name: 'a.pdf', size: 1 } },
      { type: 'upload/preview-error', message: '预览已过期' },
    )
    expect(state.upload.previewStatus).toBe('error')
    expect(state.upload.result).not.toBeNull()
    state = appReducer(state, { type: 'upload/reset' })
    expect(state.upload.phase).toBe('empty')
  })
})

describe('打印机与默认选项', () => {
  it('selects the first printer and derives A4 defaults', () => {
    const state = reduce({
      type: 'printers/loaded',
      printers: [printer('p1', '一号'), printer('p2', '二号')],
    })
    expect(state.selectedPrinterId).toBe('p1')
    expect(state.options.media).toBe('iso_a4_210x297mm')
  })

  it('keeps the current selection across refreshes and resets options on switch', () => {
    let state = reduce(
      { type: 'printers/loaded', printers: [printer('p1', '一号'), printer('p2', '二号')] },
      { type: 'options/set', key: 'media', value: 'na_letter_8.5x11in' },
    )
    state = appReducer(state, { type: 'printers/loaded', printers: [printer('p1', '一号')] })
    expect(state.selectedPrinterId).toBe('p1')
    expect(state.options.media).toBe('na_letter_8.5x11in')

    state = appReducer(state, { type: 'printer/select', id: 'p1' })
    expect(state.options.media).toBe('iso_a4_210x297mm')
  })
})

describe('任务视图', () => {
  it('merges updates, counts statuses and clears finished jobs', () => {
    let state = reduce({
      type: 'jobs/upsert',
      jobs: [job({ id: 'a' }), job({ id: 'b', status: 'completed' })],
    })
    expect(state.jobs.views).toHaveLength(2)
    expect(deriveJobCounts(state.jobs.views)).toMatchObject({ total: 2, active: 1, completed: 1 })

    state = appReducer(state, {
      type: 'jobs/upsert',
      jobs: [job({ id: 'a', status: 'completed' })],
    })
    expect(deriveJobCounts(state.jobs.views).active).toBe(0)

    state = appReducer(state, { type: 'jobs/clear-finished' })
    expect(state.jobs.views).toEqual([])
  })

  it('prunes jobs older than the retention window', () => {
    const now = 10 * MAX_JOB_AGE_MS
    const stale = job({ id: 'stale', status: 'completed', created_at_ms: 1 })
    const fresh = job({ id: 'fresh', status: 'completed', created_at_ms: now - 1000 })
    const active = job({ id: 'active', status: 'printing', created_at_ms: 1 })
    const pruned = pruneJobs([stale, fresh, active], now)
    expect(pruned.map((view) => view.id).sort()).toEqual(['active', 'fresh'])
    expect(isActiveJobStatus(active.status)).toBe(true)
  })
})

describe('步骤与打印尝试', () => {
  it('derives the three flow steps', () => {
    expect(deriveSteps(createInitialState('t'))).toEqual(['active', 'todo', 'todo'])

    const uploaded = reduce({
      type: 'upload/success',
      result: { id: 'f', name: 'a.pdf', size: 1 },
    })
    expect(deriveSteps(uploaded)).toEqual(['done', 'active', 'todo'])

    const printing = appReducer(uploaded, {
      type: 'jobs/upsert',
      jobs: [job({ status: 'printing' })],
    })
    expect(deriveSteps(printing)).toEqual(['done', 'done', 'active'])

    const done = appReducer(uploaded, { type: 'jobs/upsert', jobs: [job({ status: 'completed' })] })
    expect(deriveSteps(done)).toEqual(['done', 'done', 'done'])
  })

  it('settles the idempotency key after a successful submit', () => {
    const payload = { file_id: 'f', printer_id: 'p', options: {} }
    const { attempt } = resolveAttempt(null, printFingerprint(payload), () => 'key-1')
    let state = reduce({ type: 'print/begin', attempt })
    expect(state.print.submitting).toBe(true)
    expect(state.print.attempt?.key).toBe('key-1')

    state = appReducer(state, { type: 'print/settled' })
    expect(state.print.submitting).toBe(false)
    expect(state.print.attempt?.settled).toBe(true)
  })
})
