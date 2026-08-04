// 后端 API 封装：统一 Bearer 认证、错误解析与类型定义。

export const TOKEN_KEY = 'just_print_token'

export interface UploadResult {
  id: string
  name: string
  size: number
}

export interface CapabilityView {
  default: string | null
  kind: 'enumerated' | 'range'
  values?: string[]
  min?: number
  max?: number
}

export interface Printer {
  id: string
  name: string
  manufacturer: string | null
  serial: string | null
  pdf_supported: boolean
  postscript_supported: boolean
  pcl_supported: boolean
  capabilities: Record<string, CapabilityView> | null
}

export interface PrinterList {
  printers: Printer[]
}

export interface PrintResult {
  job_id: string
}

export type JobStatus = 'queued' | 'printing' | 'success' | 'failed'

export interface Job {
  id: string
  printer_id: string
  status: JobStatus
  error: string | null
  created_at_ms: number
}

export class ApiError extends Error {
  readonly status: number
  readonly code: string

  constructor(status: number, code: string, message: string) {
    super(message)
    this.name = 'ApiError'
    this.status = status
    this.code = code
  }
}

export function getStoredToken(): string {
  return sessionStorage.getItem(TOKEN_KEY) ?? ''
}

export function storeToken(token: string): void {
  sessionStorage.setItem(TOKEN_KEY, token)
}

export function clearStoredToken(): void {
  sessionStorage.removeItem(TOKEN_KEY)
}

export function isUnauthorized(error: unknown): error is ApiError {
  return error instanceof ApiError && error.status === 401
}

export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

async function readError(response: Response): Promise<{ code: string; message: string }> {
  try {
    const body = (await response.json()) as {
      error?: { code?: unknown; message?: unknown }
    }
    const code = typeof body.error?.code === 'string' ? body.error.code : 'internal'
    const message =
      typeof body.error?.message === 'string'
        ? body.error.message
        : `请求失败（HTTP ${response.status}）`
    return { code, message }
  } catch {
    return { code: 'internal', message: `请求失败（HTTP ${response.status}）` }
  }
}

async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const headers = new Headers(init.headers)
  headers.set('Authorization', `Bearer ${getStoredToken()}`)
  const response = await fetch(path, { ...init, headers })
  if (response.status === 401) {
    clearStoredToken()
    throw new ApiError(401, 'unauthorized', '访问令牌无效或已过期')
  }
  if (!response.ok) {
    const { code, message } = await readError(response)
    throw new ApiError(response.status, code, message)
  }
  return (await response.json()) as T
}

export async function uploadFile(file: File): Promise<UploadResult> {
  const form = new FormData()
  form.append('file', file)
  return request<UploadResult>('/api/files', { method: 'POST', body: form })
}

export async function listPrinters(): Promise<PrinterList> {
  return request<PrinterList>('/api/printers')
}

export async function submitPrint(payload: {
  file_id: string
  printer_id: string
  controls: Record<string, string>
}): Promise<PrintResult> {
  return request<PrintResult>('/api/print', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
  })
}

export async function getJob(jobId: string): Promise<Job> {
  return request<Job>(`/api/jobs/${jobId}`)
}

export async function fetchPreview(fileId: string): Promise<Blob> {
  const response = await fetch(`/api/files/${fileId}`, {
    headers: { Authorization: `Bearer ${getStoredToken()}` },
  })
  if (response.status === 401) {
    clearStoredToken()
    throw new ApiError(401, 'unauthorized', '访问令牌无效或已过期')
  }
  if (!response.ok) {
    const { code, message } = await readError(response)
    throw new ApiError(response.status, code, message)
  }
  return response.blob()
}
