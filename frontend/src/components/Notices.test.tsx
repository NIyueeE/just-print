import { act, render, screen } from '@testing-library/preact'
import { useEffect } from 'preact/hooks'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { AppStateProvider, createNotice, useAppDispatch } from '../state'
import { AUTO_DISMISS_MS, Notices } from './Notices'

function Harness() {
  const dispatch = useAppDispatch()
  useEffect(() => {
    dispatch({ type: 'notice/add', notice: createNotice('error', '打印失败，请重试') })
    dispatch({ type: 'notice/add', notice: createNotice('success', '上传成功') })
  }, [dispatch])
  return <Notices />
}

afterEach(() => {
  vi.useRealTimers()
})

describe('Notices', () => {
  it('marks errors as alerts (no auto dismiss) and successes as status (auto dismiss)', () => {
    vi.useFakeTimers()
    render(
      <AppStateProvider>
        <Harness />
      </AppStateProvider>,
    )

    expect(screen.getByText('打印失败，请重试')).toBeInTheDocument()
    expect(screen.getByRole('alert')).toHaveTextContent('打印失败，请重试')
    expect(screen.getByRole('status')).toHaveTextContent('上传成功')

    act(() => {
      vi.advanceTimersByTime(AUTO_DISMISS_MS + 100)
    })

    expect(screen.getByText('打印失败，请重试')).toBeInTheDocument()
    expect(screen.queryByText('上传成功')).not.toBeInTheDocument()
  })

  it('offers an explicit dismiss button for every notice', () => {
    render(
      <AppStateProvider>
        <Harness />
      </AppStateProvider>,
    )
    expect(screen.getByRole('button', { name: '关闭错误提示' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '关闭提示' })).toBeInTheDocument()
  })
})
