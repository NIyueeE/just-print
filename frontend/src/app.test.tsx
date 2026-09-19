import { fireEvent, render, screen } from '@testing-library/preact'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { App } from './app'
import { AppStateProvider } from './state'

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  })
}

beforeEach(() => {
  sessionStorage.setItem('just_print_token', 'test-token')
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input)
      if (url.startsWith('/api/formats')) {
        return json({ extensions: ['pdf', 'docx'], max_upload_bytes: 1024 })
      }
      if (url.startsWith('/api/printers')) {
        return json({ printers: [] })
      }
      if (url.startsWith('/api/jobs')) {
        return json({ jobs: [] })
      }
      return new Response('', { status: 404 })
    }),
  )
})

afterEach(() => {
  vi.unstubAllGlobals()
  sessionStorage.clear()
})

describe('App 集成', () => {
  it('loads formats, renders the three-step flow and returns to the gate on logout', async () => {
    render(
      <AppStateProvider>
        <App />
      </AppStateProvider>,
    )

    expect(screen.getByRole('heading', { name: '上传文档' })).toBeInTheDocument()
    expect(screen.getByRole('heading', { name: '打印' })).toBeInTheDocument()
    expect(await screen.findByText(/未发现打印机/)).toBeInTheDocument()
    expect(await screen.findByText(/支持 2 种扩展名格式/)).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: /退出登录/ }))
    expect(screen.getByText('请输入访问令牌以使用打印服务')).toBeInTheDocument()
    expect(sessionStorage.getItem('just_print_token')).toBeNull()
  })
})
