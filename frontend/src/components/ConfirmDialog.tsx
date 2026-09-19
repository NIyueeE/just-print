import type { ComponentChildren } from 'preact'
import { useEffect, useId, useRef } from 'preact/hooks'
import { AlertIcon, SpinnerIcon, XIcon } from '../icons'
import './ConfirmDialog.css'

interface ConfirmDialogProps {
  open: boolean
  title: string
  description?: string
  confirmLabel: string
  busyLabel?: string
  cancelLabel?: string
  busy?: boolean
  error?: string | null
  onConfirm: () => void
  onCancel: () => void
  children?: ComponentChildren
}

const FOCUSABLE_SELECTOR =
  'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'

/**
 * 模态确认对话框：role="dialog" + aria-modal，焦点陷阱、Esc 关闭、关闭后还原焦点。
 * 用于打印这类不可逆操作，提交前展示打印机/文件/份数/单双面/纸张/分辨率摘要。
 */
export function ConfirmDialog({
  open,
  title,
  description,
  confirmLabel,
  busyLabel = '提交中…',
  cancelLabel = '取消',
  busy = false,
  error = null,
  onConfirm,
  onCancel,
  children,
}: ConfirmDialogProps) {
  const titleId = useId()
  const descriptionId = useId()
  const dialogRef = useRef<HTMLDivElement>(null)
  const previousFocusRef = useRef<HTMLElement | null>(null)
  const onCancelRef = useRef(onCancel)
  const busyRef = useRef(busy)
  onCancelRef.current = onCancel
  busyRef.current = busy

  useEffect(() => {
    if (!open) {
      return
    }
    const node = dialogRef.current
    previousFocusRef.current =
      document.activeElement instanceof HTMLElement ? document.activeElement : null

    const focusable = (): HTMLElement[] =>
      node === null ? [] : Array.from(node.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR))

    const initial = focusable()
    if (initial.length > 0) {
      initial[0].focus()
    } else {
      node?.focus()
    }

    function handleKeyDown(event: KeyboardEvent): void {
      if (event.key === 'Escape') {
        event.preventDefault()
        if (!busyRef.current) {
          onCancelRef.current()
        }
        return
      }
      if (event.key !== 'Tab') {
        return
      }
      const items = focusable()
      if (items.length === 0) {
        event.preventDefault()
        node?.focus()
        return
      }
      const first = items[0]
      const last = items[items.length - 1]
      const active = document.activeElement
      const inside = node !== null && active instanceof Node && node.contains(active)
      if (event.shiftKey && (active === first || !inside)) {
        event.preventDefault()
        last.focus()
      } else if (!event.shiftKey && (active === last || !inside)) {
        event.preventDefault()
        first.focus()
      }
    }

    document.addEventListener('keydown', handleKeyDown)
    return () => {
      document.removeEventListener('keydown', handleKeyDown)
      previousFocusRef.current?.focus()
    }
  }, [open])

  if (!open) {
    return null
  }

  return (
    <div class="dialog">
      <div class="dialog__backdrop" aria-hidden="true" />
      <div
        ref={dialogRef}
        class="dialog__panel"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={description !== undefined ? descriptionId : undefined}
        aria-busy={busy}
        tabIndex={-1}
      >
        <div class="dialog__header">
          <h2 class="dialog__title" id={titleId}>
            {title}
          </h2>
          <button
            type="button"
            class="dialog__close"
            aria-label="关闭对话框"
            onClick={onCancel}
            disabled={busy}
          >
            <XIcon size={16} />
          </button>
        </div>
        {description !== undefined ? (
          <p class="dialog__description" id={descriptionId}>
            {description}
          </p>
        ) : null}
        {children !== undefined ? <div class="dialog__body">{children}</div> : null}
        {error !== null ? (
          <div class="dialog__error" role="alert">
            <AlertIcon size={16} />
            <span>{error}</span>
          </div>
        ) : null}
        <div class="dialog__actions">
          <button type="button" class="ghost" onClick={onCancel} disabled={busy}>
            {cancelLabel}
          </button>
          <button type="button" class="primary" onClick={onConfirm} disabled={busy}>
            {busy ? (
              <>
                <SpinnerIcon size={16} />
                {busyLabel}
              </>
            ) : (
              confirmLabel
            )}
          </button>
        </div>
      </div>
    </div>
  )
}
