import { describe, expect, it, vi } from 'vitest'
import { ApiError, type JobView } from '../api'
import { refreshPendingJobs, TERMINAL_LOOKUP_CONCURRENCY } from './JobList'

function job(id: string): JobView {
  return {
    id,
    printer_id: 'CUPS-PDF',
    name: null,
    file_id: 'file-1',
    file_name: 'report.pdf',
    status: 'printing',
    error: null,
    created_at_ms: Date.now(),
    options: {},
  }
}

const neverAborted = new AbortController().signal

describe('任务终态补查', () => {
  it('updates to the fresh state and maps CUPS cleanup to unknown', async () => {
    const lookup = vi.fn(async (id: string) => {
      if (id === 'gone') {
        throw new ApiError(404, 'not_found', '任务不存在')
      }
      return { ...job(id), status: 'completed' as const }
    })

    const refreshed = await refreshPendingJobs([job('done'), job('gone')], neverAborted, lookup)
    expect(refreshed.map((item) => [item.id, item.status])).toEqual([
      ['done', 'completed'],
      ['gone', null],
    ])
    expect(refreshed[1].file_id).toBe('file-1')
  })

  it('propagates transient errors so the polling layer can back off and retry', async () => {
    const failure = new ApiError(503, 'service_unavailable', 'CUPS 不可用')
    await expect(
      refreshPendingJobs([job('active')], neverAborted, async () => {
        throw failure
      }),
    ).rejects.toBe(failure)
  })

  it('在补查多个任务时限制并发', async () => {
    let active = 0
    let peak = 0
    const pending = Array.from({ length: 9 }, (_, index) => job(`j${index}`))
    const refreshed = await refreshPendingJobs(pending, neverAborted, async (id) => {
      active += 1
      peak = Math.max(peak, active)
      await Promise.resolve()
      active -= 1
      return { ...job(id), status: 'completed' }
    })
    expect(refreshed).toHaveLength(9)
    expect(peak).toBeLessThanOrEqual(TERMINAL_LOOKUP_CONCURRENCY)
  })
})
