import { fireEvent, render, screen, within } from '@testing-library/preact'
import { describe, expect, it, vi } from 'vitest'
import { ConfirmDialog } from './ConfirmDialog'

describe('ConfirmDialog 可访问性', () => {
  it('is a labelled modal dialog that moves focus inside', () => {
    render(
      <ConfirmDialog
        open
        title="确认打印"
        description="打印是物理操作"
        confirmLabel="确认打印"
        onConfirm={() => undefined}
        onCancel={() => undefined}
      >
        <p>摘要</p>
      </ConfirmDialog>,
    )

    const dialog = screen.getByRole('dialog')
    expect(dialog).toHaveAttribute('aria-modal', 'true')
    expect(dialog).toHaveAccessibleName('确认打印')
    expect(dialog).toHaveAccessibleDescription('打印是物理操作')
    expect(dialog.contains(document.activeElement)).toBe(true)
  })

  it('closes on Escape and traps Tab inside the dialog', () => {
    const onCancel = vi.fn()
    render(
      <ConfirmDialog
        open
        title="确认打印"
        confirmLabel="确认打印"
        onConfirm={() => undefined}
        onCancel={onCancel}
      />,
    )

    const dialog = screen.getByRole('dialog')
    const buttons = within(dialog).getAllByRole('button')
    const first = buttons[0]
    const last = buttons[buttons.length - 1]

    last.focus()
    fireEvent.keyDown(document, { key: 'Tab' })
    expect(document.activeElement).toBe(first)

    fireEvent.keyDown(document, { key: 'Tab', shiftKey: true })
    expect(document.activeElement).toBe(last)

    fireEvent.keyDown(document, { key: 'Escape' })
    expect(onCancel).toHaveBeenCalledTimes(1)
  })

  it('does not render when closed', () => {
    render(
      <ConfirmDialog
        open={false}
        title="确认打印"
        confirmLabel="确认打印"
        onConfirm={() => undefined}
        onCancel={() => undefined}
      />,
    )
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
  })

  it('locks page scrolling while open and restores it on unmount', () => {
    const { unmount } = render(
      <ConfirmDialog
        open
        title="确认打印"
        confirmLabel="确认打印"
        onConfirm={() => undefined}
        onCancel={() => undefined}
      />,
    )
    // 移动端在遮罩上滑动不应该滚动背后的页面。
    expect(document.body.style.overflow).toBe('hidden')

    unmount()
    expect(document.body.style.overflow).toBe('')
  })
})
