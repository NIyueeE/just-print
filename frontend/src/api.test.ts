import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  ApiError,
  isIdempotencyConflict,
  isRetryable,
  listPrinters,
  parseErrorPayload,
  parseRetryAfter,
  retryAfterHint,
  submitPrint,
  getFormats,
} from './api'

function jsonResponse(
  body: unknown,
  status: number,
  headers: Record<string, string> = {},
): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json', ...headers },
  })
}

beforeEach(() => {
  sessionStorage.clear()
  sessionStorage.setItem('just_print_token', 'test-token')
})

afterEach(() => {
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

describe('错误信封解析', () => {
  it('reads code and message from the backend envelope', () => {
    expect(
      parseErrorPayload(
        400,
        JSON.stringify({ error: { code: 'invalid_controls', message: '选项不合法' } }),
      ),
    ).toEqual({ code: 'invalid_controls', message: '选项不合法' })
  })

  it('maps unknown codes to the HTTP status and falls back on non-JSON bodies', () => {
    expect(parseErrorPayload(503, JSON.stringify({ error: { code: 'wat', message: '' } }))).toEqual(
      {
        code: 'service_unavailable',
        message: '请求失败（HTTP 503）',
      },
    )
    expect(parseErrorPayload(500, '<html>boom</html>')).toEqual({
      code: 'internal',
      message: '请求失败（HTTP 500）',
    })
  })
})

describe('Retry-After 与可重试判定', () => {
  it('parses seconds and HTTP dates with an upper bound', () => {
    expect(parseRetryAfter('2')).toBe(2000)
    expect(parseRetryAfter('0')).toBe(0)
    expect(parseRetryAfter('9999')).toBe(60_000)
    expect(parseRetryAfter('not-a-date')).toBeNull()
    expect(parseRetryAfter(null)).toBeNull()
    const future = new Date(Date.now() + 5000).toUTCString()
    const parsed = parseRetryAfter(future)
    expect(parsed).not.toBeNull()
    expect(parsed ?? 0).toBeGreaterThan(1000)
  })

  it('marks gateway/network errors retryable but not validation errors', () => {
    expect(isRetryable(new ApiError(0, 'network', 'offline'))).toBe(true)
    expect(isRetryable(new ApiError(0, 'timeout', 'timeout'))).toBe(true)
    expect(isRetryable(new ApiError(503, 'service_unavailable', 'busy'))).toBe(true)
    expect(isRetryable(new ApiError(504, 'gateway_timeout', 'slow'))).toBe(true)
    expect(isRetryable(new ApiError(400, 'invalid_controls', 'bad'))).toBe(false)
    expect(isRetryable(new Error('boom'))).toBe(false)
    expect(isIdempotencyConflict(new ApiError(409, 'idempotency_conflict', 'busy'))).toBe(true)
  })

  it('renders a Retry-After hint only when the server asked to wait', () => {
    expect(retryAfterHint(new ApiError(503, 'service_unavailable', 'busy', 3000))).toContain('3 秒')
    expect(retryAfterHint(new ApiError(503, 'service_unavailable', 'busy'))).toBe('')
    expect(retryAfterHint(new Error('x'))).toBe('')
  })
})

describe('请求行为', () => {
  it('sends Idempotency-Key and Authorization, and reports replays', async () => {
    const fetchMock = vi.fn(async (_input: RequestInfo | URL, _init?: RequestInit) =>
      jsonResponse(
        {
          job_id: 'CUPS-PDF-8',
          printer_id: 'CUPS-PDF',
          status: 'queued',
          created_at_ms: 1,
          file_name: 'report.pdf',
        },
        202,
        { 'Idempotency-Replayed': 'true' },
      ),
    )
    vi.stubGlobal('fetch', fetchMock)

    const response = await submitPrint(
      { file_id: 'f1', printer_id: 'CUPS-PDF', options: { copies: '2' } },
      'key-1',
    )
    expect(response.replayed).toBe(true)
    expect(response.job.job_id).toBe('CUPS-PDF-8')

    const firstCall = fetchMock.mock.calls.at(0)
    expect(firstCall).toBeDefined()
    const url = firstCall?.[0]
    const init = firstCall?.[1]
    expect(url).toBe('/api/print')
    const headers = new Headers(init?.headers)
    expect(headers.get('Idempotency-Key')).toBe('key-1')
    expect(headers.get('Authorization')).toBe('Bearer test-token')
    expect(headers.get('Content-Type')).toBe('application/json')
  })

  it('turns 401 into an unauthorized ApiError and never leaks the token into the URL', async () => {
    const fetchMock = vi.fn(async (_input: RequestInfo | URL) => new Response('', { status: 401 }))
    vi.stubGlobal('fetch', fetchMock)

    await expect(listPrinters()).rejects.toBeInstanceOf(ApiError)
    const firstCall = fetchMock.mock.calls.at(0)
    expect(firstCall?.[0]).toBe('/api/printers')
  })

  it('surfaces Retry-After on 503', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () =>
        jsonResponse({ error: { code: 'service_unavailable', message: 'CUPS 不可用' } }, 503, {
          'Retry-After': '3',
        }),
      ),
    )
    await expect(listPrinters()).rejects.toMatchObject({
      code: 'service_unavailable',
      retryAfterMs: 3000,
    })
  })

  it('bypasses the server cache with refresh=true and unwraps formats', async () => {
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input)
      if (url.startsWith('/api/formats')) {
        return jsonResponse({ extensions: ['pdf', 'docx'], max_upload_bytes: 1024 }, 200)
      }
      return jsonResponse({ printers: [] }, 200)
    })
    vi.stubGlobal('fetch', fetchMock)

    await listPrinters({ refresh: true })
    expect(fetchMock.mock.calls.at(0)?.[0]).toBe('/api/printers?refresh=true')

    await expect(getFormats()).resolves.toEqual({
      extensions: ['pdf', 'docx'],
      max_upload_bytes: 1024,
    })
  })

  it('maps transport failures to a network ApiError', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => {
        throw new TypeError('Failed to fetch')
      }),
    )
    await expect(listPrinters()).rejects.toMatchObject({ code: 'network', status: 0 })
  })
})
