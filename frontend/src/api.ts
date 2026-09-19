/* =============================================================================
 * 后端 API 单一客户端。
 *
 * 约定：
 *   - 所有请求同源，统一携带 `Authorization: Bearer <token>`；
 *   - 令牌只保存在 sessionStorage（标签页级、关闭即失效，比 localStorage 暴露面小），
 *     绝不放进 URL/查询参数，也不写入日志；
 *   - 所有请求都通过 AbortController 支持外部取消 + 超时；幂等 GET 的重试策略由
 *     调用方依据 `isRetryable` 决定；
 *   - 错误统一为 `ApiError`，解析 `{"error":{"code","message"}}` 信封。
 * ========================================================================== */

/** sessionStorage 中的令牌键名。 */
export const TOKEN_STORAGE_KEY = 'just_print_token'

/** 普通 JSON 请求默认超时。 */
export const DEFAULT_TIMEOUT_MS = 20_000

/** 预览文件可能较大，给更宽松的超时。 */
export const PREVIEW_TIMEOUT_MS = 60_000

/** Retry-After 的最长等待，避免 UI 被服务端要求长时间挂起。 */
export const MAX_RETRY_AFTER_MS = 60_000

/* -------------------------------------------------------------------------- */
/* 线上类型（与后端 JSON 契约一一对应）                                        */
/* -------------------------------------------------------------------------- */

export interface UploadResult {
  id: string
  name: string
  size: number
}

export interface Formats {
  extensions: string[]
  max_upload_bytes: number
}

export interface PrinterList {
  printers: Printer[]
}

export type PrinterState = 'idle' | 'printing' | 'stopped' | 'disabled'

export interface EnumOptionValue {
  value: number
  name: string
}

export interface ResolutionValue {
  cross_feed: number
  feed: number
  units: number
  label: string
}

export type OptionSpec =
  | { kind: 'keyword'; default: string | null; values: string[] }
  | { kind: 'enum'; default: string | null; values: EnumOptionValue[] }
  | { kind: 'integer'; default: string | null; min: number; max: number }
  | { kind: 'integer_choices'; default: string | null; values: number[] }
  | { kind: 'resolution'; default: string | null; values: ResolutionValue[] }

export interface Printer {
  id: string
  name: string
  state: PrinterState
  accepting_jobs: boolean
  make_and_model: string | null
  location: string | null
  options: Record<string, OptionSpec>
  options_error: string | null
}

export type JobStatus = 'queued' | 'printing' | 'completed' | 'failed' | 'canceled'

export interface JobView {
  id: string
  printer_id: string
  name: string | null
  file_id: string | null
  file_name: string | null
  status: JobStatus | null
  error: string | null
  created_at_ms: number
  options: Record<string, string>
}

export interface JobListResponse {
  jobs: JobView[]
}

export interface PrintAccepted {
  job_id: string
  printer_id: string
  status: JobStatus
  created_at_ms: number
  file_name: string
}

export interface PrintPayload {
  file_id: string
  printer_id: string
  options: Record<string, string>
}

export interface PrintResponse {
  job: PrintAccepted
  /** 服务端回放了相同幂等键的首次响应。 */
  replayed: boolean
}

/** 后端错误码 + 两个客户端侧错误码。 */
export type ApiErrorCode =
  | 'unauthorized'
  | 'bad_request'
  | 'not_found'
  | 'unsupported_media_type'
  | 'payload_too_large'
  | 'conversion_failed'
  | 'printer_unavailable'
  | 'conflict'
  | 'idempotency_conflict'
  | 'invalid_controls'
  | 'bad_gateway'
  | 'service_unavailable'
  | 'gateway_timeout'
  | 'internal'
  | 'network'
  | 'timeout'

const KNOWN_ERROR_CODES: readonly string[] = [
  'unauthorized',
  'bad_request',
  'not_found',
  'unsupported_media_type',
  'payload_too_large',
  'conversion_failed',
  'printer_unavailable',
  'conflict',
  'idempotency_conflict',
  'invalid_controls',
  'bad_gateway',
  'service_unavailable',
  'gateway_timeout',
  'internal',
]

const FALLBACK_MESSAGES: Partial<Record<ApiErrorCode, string>> = {
  unauthorized: '访问令牌无效或已过期',
  network: '无法连接到服务器，请检查网络后重试',
  timeout: '请求超时，请重试',
}

/* -------------------------------------------------------------------------- */
/* 错误                                                                        */
/* -------------------------------------------------------------------------- */

export class ApiError extends Error {
  readonly status: number
  readonly code: ApiErrorCode
  /** 服务端 `Retry-After` 换算出的毫秒数（已按上限截断）。 */
  readonly retryAfterMs: number | null

  constructor(
    status: number,
    code: ApiErrorCode,
    message: string,
    retryAfterMs: number | null = null,
  ) {
    super(message)
    this.name = 'ApiError'
    this.status = status
    this.code = code
    this.retryAfterMs = retryAfterMs
  }

  get isUnauthorized(): boolean {
    return this.status === 401 || this.code === 'unauthorized'
  }

  get isIdempotencyConflict(): boolean {
    return this.code === 'idempotency_conflict'
  }
}

export function isUnauthorized(error: unknown): boolean {
  return error instanceof ApiError && error.isUnauthorized
}

export function isIdempotencyConflict(error: unknown): boolean {
  return error instanceof ApiError && error.isIdempotencyConflict
}

/** 网络抖动或网关类错误可以安全重试（GET 幂等；POST 需配合相同幂等键）。 */
export function isRetryable(error: unknown): boolean {
  if (!(error instanceof ApiError)) {
    return false
  }
  return (
    error.code === 'network' ||
    error.code === 'timeout' ||
    error.status === 502 ||
    error.status === 503 ||
    error.status === 504
  )
}

export function isAbortError(error: unknown): boolean {
  return error instanceof DOMException ? error.name === 'AbortError' : false
}

export function errorMessage(error: unknown): string {
  if (error instanceof Error) {
    return error.message
  }
  return String(error)
}

/** 把服务端 `Retry-After` 渲染成可读提示，追加到错误文案后。 */
export function retryAfterHint(error: unknown): string {
  if (error instanceof ApiError && error.retryAfterMs !== null && error.retryAfterMs > 0) {
    return `（服务端建议约 ${Math.ceil(error.retryAfterMs / 1000)} 秒后重试）`
  }
  return ''
}

function fallbackMessage(code: ApiErrorCode, status: number): string {
  return FALLBACK_MESSAGES[code] ?? `请求失败（HTTP ${status}）`
}

function codeForStatus(status: number): ApiErrorCode {
  switch (status) {
    case 400:
      return 'bad_request'
    case 401:
      return 'unauthorized'
    case 404:
      return 'not_found'
    case 409:
      return 'conflict'
    case 413:
      return 'payload_too_large'
    case 415:
      return 'unsupported_media_type'
    case 422:
      return 'conversion_failed'
    case 500:
      return 'internal'
    case 502:
      return 'bad_gateway'
    case 503:
      return 'service_unavailable'
    case 504:
      return 'gateway_timeout'
    default:
      return 'internal'
  }
}

/** 解析错误响应体；非 JSON 或字段缺失时回退到状态码映射。 */
export function parseErrorPayload(
  status: number,
  body: string,
): { code: ApiErrorCode; message: string } {
  const fallbackCode = codeForStatus(status)
  if (body.length === 0) {
    return { code: fallbackCode, message: fallbackMessage(fallbackCode, status) }
  }
  try {
    const parsed: unknown = JSON.parse(body)
    if (typeof parsed === 'object' && parsed !== null && 'error' in parsed) {
      const envelope = (parsed as { error: unknown }).error
      if (typeof envelope === 'object' && envelope !== null) {
        const rawCode = (envelope as { code?: unknown }).code
        const rawMessage = (envelope as { message?: unknown }).message
        const code =
          typeof rawCode === 'string' && KNOWN_ERROR_CODES.includes(rawCode)
            ? (rawCode as ApiErrorCode)
            : fallbackCode
        const message =
          typeof rawMessage === 'string' && rawMessage.trim().length > 0
            ? rawMessage
            : fallbackMessage(code, status)
        return { code, message }
      }
    }
  } catch {
    /* 响应体不是 JSON，使用状态码回退。 */
  }
  return { code: fallbackCode, message: fallbackMessage(fallbackCode, status) }
}

/** 解析 `Retry-After`（秒数或 HTTP 日期）为毫秒，并截断到上限。 */
export function parseRetryAfter(value: string | null): number | null {
  if (value === null) {
    return null
  }
  const trimmed = value.trim()
  if (trimmed.length === 0) {
    return null
  }
  if (/^\d+$/.test(trimmed)) {
    return Math.min(Number(trimmed) * 1000, MAX_RETRY_AFTER_MS)
  }
  const timestamp = Date.parse(trimmed)
  if (Number.isNaN(timestamp)) {
    return null
  }
  return Math.min(Math.max(timestamp - Date.now(), 0), MAX_RETRY_AFTER_MS)
}

function createAbortError(): DOMException {
  return new DOMException('请求已取消', 'AbortError')
}

/* -------------------------------------------------------------------------- */
/* 令牌存取                                                                    */
/* -------------------------------------------------------------------------- */

export function getStoredToken(): string {
  return sessionStorage.getItem(TOKEN_STORAGE_KEY) ?? ''
}

export function storeToken(token: string): void {
  sessionStorage.setItem(TOKEN_STORAGE_KEY, token)
}

export function clearStoredToken(): void {
  sessionStorage.removeItem(TOKEN_STORAGE_KEY)
}

/* -------------------------------------------------------------------------- */
/* 请求基础设施                                                                */
/* -------------------------------------------------------------------------- */

interface LinkedAbort {
  signal: AbortSignal
  timedOut: () => boolean
  dispose: () => void
}

function createLinkedAbort(timeoutMs: number, external?: AbortSignal): LinkedAbort {
  const controller = new AbortController()
  let timedOut = false
  let timer: number | undefined

  const forwardAbort = (): void => controller.abort()
  if (external) {
    if (external.aborted) {
      controller.abort()
    } else {
      external.addEventListener('abort', forwardAbort, { once: true })
    }
  }
  if (timeoutMs > 0 && !controller.signal.aborted) {
    timer = window.setTimeout(() => {
      timedOut = true
      controller.abort()
    }, timeoutMs)
  }

  return {
    signal: controller.signal,
    timedOut: () => timedOut,
    dispose: () => {
      if (timer !== undefined) {
        window.clearTimeout(timer)
      }
      external?.removeEventListener('abort', forwardAbort)
    },
  }
}

function normalizeTransportError(
  error: unknown,
  abort: LinkedAbort,
  external?: AbortSignal,
): unknown {
  if (error instanceof ApiError) {
    return error
  }
  if (isAbortError(error)) {
    if (external?.aborted) {
      return createAbortError()
    }
    if (abort.timedOut()) {
      return new ApiError(0, 'timeout', fallbackMessage('timeout', 0))
    }
    return createAbortError()
  }
  if (error instanceof TypeError) {
    return new ApiError(0, 'network', fallbackMessage('network', 0))
  }
  return error
}

async function toApiError(response: Response): Promise<ApiError> {
  let body: string
  try {
    body = await response.text()
  } catch {
    body = ''
  }
  const { code, message } = parseErrorPayload(response.status, body)
  return new ApiError(
    response.status,
    code,
    message,
    parseRetryAfter(response.headers.get('Retry-After')),
  )
}

interface RequestOptions {
  method?: string
  body?: BodyInit
  headers?: HeadersInit
  signal?: AbortSignal
  /** 覆盖默认令牌（仅用于登录校验）。 */
  token?: string
  timeoutMs?: number
  idempotencyKey?: string
  accept?: string
}

async function requestJson<T>(
  path: string,
  options: RequestOptions = {},
): Promise<{ data: T; response: Response }> {
  const {
    method = 'GET',
    body,
    headers: extraHeaders,
    signal,
    token,
    timeoutMs = DEFAULT_TIMEOUT_MS,
    idempotencyKey,
    accept = 'application/json',
  } = options

  const abort = createLinkedAbort(timeoutMs, signal)
  try {
    const headers = new Headers(extraHeaders)
    headers.set('Authorization', `Bearer ${token ?? getStoredToken()}`)
    headers.set('Accept', accept)
    if (idempotencyKey !== undefined) {
      headers.set('Idempotency-Key', idempotencyKey)
    }
    const response = await fetch(path, {
      method,
      body,
      headers,
      signal: abort.signal,
      credentials: 'same-origin',
    })
    if (response.status === 401) {
      throw new ApiError(401, 'unauthorized', fallbackMessage('unauthorized', 401))
    }
    if (!response.ok) {
      throw await toApiError(response)
    }
    if (response.status === 204) {
      return { data: undefined as T, response }
    }
    const data = (await response.json()) as T
    return { data, response }
  } catch (error) {
    throw normalizeTransportError(error, abort, signal)
  } finally {
    abort.dispose()
  }
}

function jsonBody(payload: unknown): BodyInit {
  return JSON.stringify(payload)
}

/* -------------------------------------------------------------------------- */
/* 具体接口                                                                    */
/* -------------------------------------------------------------------------- */

/** 登录校验：用显式令牌探测 `/api/printers`，成功后再落盘。 */
export async function validateToken(token: string, signal?: AbortSignal): Promise<void> {
  await requestJson<PrinterList>('/api/printers', { token, signal, timeoutMs: 10_000 })
}

export async function getFormats(signal?: AbortSignal): Promise<Formats> {
  const { data } = await requestJson<Formats>('/api/formats', { signal })
  return data
}

export async function listPrinters(
  options: { refresh?: boolean; signal?: AbortSignal } = {},
): Promise<Printer[]> {
  const query = options.refresh ? '?refresh=true' : ''
  const { data } = await requestJson<PrinterList>(`/api/printers${query}`, {
    signal: options.signal,
  })
  return data.printers
}

export interface UploadOptions {
  onProgress?: (percent: number) => void
  signal?: AbortSignal
}

/**
 * 上传并转换文件。
 * 使用 XMLHttpRequest：`fetch` 无法提供上传进度；取消通过 signal → xhr.abort() 实现。
 */
export function uploadFile(file: File, options: UploadOptions = {}): Promise<UploadResult> {
  return new Promise<UploadResult>((resolve, reject) => {
    const xhr = new XMLHttpRequest()
    const form = new FormData()
    form.append('file', file, file.name)
    let settled = false

    const onAbort = (): void => xhr.abort()
    const cleanup = (): void => {
      options.signal?.removeEventListener('abort', onAbort)
    }
    const fail = (error: unknown): void => {
      if (settled) return
      settled = true
      cleanup()
      reject(error)
    }
    const succeed = (value: UploadResult): void => {
      if (settled) return
      settled = true
      cleanup()
      resolve(value)
    }

    xhr.open('POST', '/api/files')
    xhr.setRequestHeader('Authorization', `Bearer ${getStoredToken()}`)
    xhr.setRequestHeader('Accept', 'application/json')
    xhr.responseType = 'text'
    xhr.upload.onprogress = (event: ProgressEvent): void => {
      if (event.lengthComputable && event.total > 0) {
        options.onProgress?.(Math.min(100, Math.round((event.loaded / event.total) * 100)))
      }
    }
    xhr.onload = (): void => {
      if (xhr.status === 401) {
        fail(new ApiError(401, 'unauthorized', fallbackMessage('unauthorized', 401)))
        return
      }
      if (xhr.status < 200 || xhr.status >= 300) {
        const { code, message } = parseErrorPayload(xhr.status, xhr.responseText)
        fail(
          new ApiError(
            xhr.status,
            code,
            message,
            parseRetryAfter(xhr.getResponseHeader('Retry-After')),
          ),
        )
        return
      }
      try {
        const parsed = JSON.parse(xhr.responseText) as UploadResult
        options.onProgress?.(100)
        succeed(parsed)
      } catch {
        fail(new ApiError(xhr.status, 'internal', '服务器返回了无法解析的响应'))
      }
    }
    xhr.onerror = (): void => fail(new ApiError(0, 'network', fallbackMessage('network', 0)))
    xhr.ontimeout = (): void => fail(new ApiError(0, 'timeout', fallbackMessage('timeout', 0)))
    xhr.onabort = (): void => fail(createAbortError())

    if (options.signal) {
      if (options.signal.aborted) {
        fail(createAbortError())
        return
      }
      options.signal.addEventListener('abort', onAbort, { once: true })
    }
    xhr.send(form)
  })
}

export async function fetchPreview(fileId: string, signal?: AbortSignal): Promise<Blob> {
  const abort = createLinkedAbort(PREVIEW_TIMEOUT_MS, signal)
  try {
    const response = await fetch(`/api/files/${encodeURIComponent(fileId)}`, {
      headers: { Authorization: `Bearer ${getStoredToken()}`, Accept: 'application/pdf' },
      signal: abort.signal,
      credentials: 'same-origin',
    })
    if (response.status === 401) {
      throw new ApiError(401, 'unauthorized', fallbackMessage('unauthorized', 401))
    }
    if (!response.ok) {
      throw await toApiError(response)
    }
    return await response.blob()
  } catch (error) {
    throw normalizeTransportError(error, abort, signal)
  } finally {
    abort.dispose()
  }
}

export async function deleteFile(fileId: string, signal?: AbortSignal): Promise<void> {
  await requestJson<void>(`/api/files/${encodeURIComponent(fileId)}`, { method: 'DELETE', signal })
}

/** 提交打印。`idempotencyKey` 由调用方按“同一逻辑打印尝试”复用。 */
export async function submitPrint(
  payload: PrintPayload,
  idempotencyKey: string,
  signal?: AbortSignal,
): Promise<PrintResponse> {
  const { data, response } = await requestJson<PrintAccepted>('/api/print', {
    method: 'POST',
    body: jsonBody(payload),
    headers: { 'Content-Type': 'application/json' },
    idempotencyKey,
    signal,
  })
  return { job: data, replayed: response.headers.get('Idempotency-Replayed') === 'true' }
}

export async function listJobs(
  options: { limit?: number; active?: boolean; signal?: AbortSignal } = {},
): Promise<JobView[]> {
  const params = new URLSearchParams()
  if (options.limit !== undefined) {
    params.set('limit', String(options.limit))
  }
  if (options.active !== undefined) {
    params.set('active', String(options.active))
  }
  const query = params.toString()
  const { data } = await requestJson<JobListResponse>(
    `/api/jobs${query.length > 0 ? `?${query}` : ''}`,
    {
      signal: options.signal,
    },
  )
  return data.jobs
}

export async function getJob(jobId: string, signal?: AbortSignal): Promise<JobView> {
  const { data } = await requestJson<JobView>(`/api/jobs/${encodeURIComponent(jobId)}`, { signal })
  return data
}

export async function cancelJob(jobId: string, signal?: AbortSignal): Promise<void> {
  await requestJson<void>(`/api/jobs/${encodeURIComponent(jobId)}`, { method: 'DELETE', signal })
}
