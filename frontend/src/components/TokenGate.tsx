import { useEffect, useRef, useState } from 'preact/hooks'
import { errorMessage, isUnauthorized, storeToken, validateToken } from '../api'
import {
  AlertIcon,
  EyeIcon,
  EyeOffIcon,
  InfoIcon,
  KeyIcon,
  LockIcon,
  LogoMark,
  SpinnerIcon,
} from '../icons'
import './TokenGate.css'

interface TokenGateProps {
  /** 因 401 被强制退出登录时的说明文案。 */
  message?: string | null
  onValid: (token: string) => void
}

export function TokenGate({ message = null, onValid }: TokenGateProps) {
  const [value, setValue] = useState('')
  const [show, setShow] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const controllerRef = useRef<AbortController | null>(null)

  useEffect(() => {
    return () => controllerRef.current?.abort()
  }, [])

  async function handleSubmit(event: Event): Promise<void> {
    event.preventDefault()
    if (busy) {
      return
    }
    const token = value.trim()
    if (token.length === 0) {
      setError('请输入访问令牌')
      return
    }
    controllerRef.current?.abort()
    const controller = new AbortController()
    controllerRef.current = controller
    setBusy(true)
    setError(null)
    try {
      await validateToken(token, controller.signal)
      storeToken(token)
      onValid(token)
    } catch (requestError) {
      if (controller.signal.aborted) {
        return
      }
      if (isUnauthorized(requestError)) {
        setError('令牌无效，请检查后重试。')
      } else {
        setError(`验证失败：${errorMessage(requestError)}`)
      }
    } finally {
      if (controllerRef.current === controller) {
        controllerRef.current = null
      }
      setBusy(false)
    }
  }

  const describedBy = [
    ...(message !== null ? ['token-message'] : []),
    ...(error !== null ? ['token-error'] : []),
  ].join(' ')

  return (
    <div class="token-gate">
      <form class="token-gate__card card" onSubmit={handleSubmit} aria-busy={busy}>
        <div class="token-gate__logo">
          <LogoMark size={76} />
        </div>
        <div>
          <h1 class="token-gate__title">Just Print</h1>
          <p class="muted">输入访问令牌</p>
        </div>
        {message !== null ? (
          <p class="token-gate__notice" id="token-message" role="status">
            <InfoIcon size={15} />
            {message}
          </p>
        ) : null}
        <div class="token-gate__field">
          <label class="visually-hidden" htmlFor="token-input">
            访问令牌
          </label>
          <KeyIcon size={17} className="token-gate__key" />
          <input
            id="token-input"
            type={show ? 'text' : 'password'}
            value={value}
            placeholder="JUST_PRINT_TOKEN"
            autocomplete="off"
            autocapitalize="off"
            spellcheck={false}
            disabled={busy}
            aria-invalid={error !== null}
            aria-describedby={describedBy}
            onInput={(event) => {
              setValue((event.target as HTMLInputElement).value)
              setError(null)
            }}
          />
          <button
            type="button"
            class="token-gate__eye"
            onClick={() => setShow((visible) => !visible)}
            aria-label={show ? '隐藏令牌' : '显示令牌'}
            title={show ? '隐藏令牌' : '显示令牌'}
          >
            {show ? <EyeOffIcon size={17} /> : <EyeIcon size={17} />}
          </button>
        </div>
        <button type="submit" class="primary" disabled={busy || value.trim().length === 0}>
          {busy ? (
            <>
              <SpinnerIcon size={16} />
              验证中…
            </>
          ) : (
            <>
              <LockIcon size={16} />
              进入
            </>
          )}
        </button>
        {error !== null ? (
          <p class="token-gate__error" id="token-error" role="alert">
            <AlertIcon size={15} />
            {error}
          </p>
        ) : null}
      </form>
    </div>
  )
}
