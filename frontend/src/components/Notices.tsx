import { useEffect } from 'preact/hooks'
import { AlertIcon, CheckIcon, InfoIcon, XIcon } from '../icons'
import { useAppDispatch, useAppState, type Notice } from '../state'
import './Notices.css'

/** 成功/信息提示自动消失；错误必须由用户显式关闭。 */
export const AUTO_DISMISS_MS = 6000

function NoticeItem({ notice }: { notice: Notice }) {
  const dispatch = useAppDispatch()

  useEffect(() => {
    if (notice.kind === 'error') {
      return
    }
    const timer = window.setTimeout(() => {
      dispatch({ type: 'notice/dismiss', id: notice.id })
    }, AUTO_DISMISS_MS)
    return () => window.clearTimeout(timer)
  }, [dispatch, notice.id, notice.kind])

  const isError = notice.kind === 'error'

  return (
    <li class={`notice notice--${notice.kind}`} role={isError ? 'alert' : 'status'}>
      <span class="notice__icon" aria-hidden="true">
        {notice.kind === 'success' ? (
          <CheckIcon size={16} />
        ) : isError ? (
          <AlertIcon size={16} />
        ) : (
          <InfoIcon size={16} />
        )}
      </span>
      <span class="notice__message">{notice.message}</span>
      <button
        type="button"
        class="notice__dismiss"
        aria-label={isError ? '关闭错误提示' : '关闭提示'}
        onClick={() => dispatch({ type: 'notice/dismiss', id: notice.id })}
      >
        <XIcon size={14} />
      </button>
    </li>
  )
}

export function Notices() {
  const { notices } = useAppState()
  if (notices.length === 0) {
    return null
  }
  return (
    <div class="notices">
      <ul class="notices__list">
        {notices.map((notice) => (
          <NoticeItem key={notice.id} notice={notice} />
        ))}
      </ul>
    </div>
  )
}
