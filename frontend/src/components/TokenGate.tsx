import { useState } from 'preact/hooks'
import { storeToken } from '../api'
import { AlertIcon, KeyIcon, LockIcon, LogoMark } from '../icons'

interface TokenGateProps {
  onValid: (token: string) => void
}

export function TokenGate({ onValid }: TokenGateProps) {
  const [value, setValue] = useState('')
  const [error, setError] = useState<string | null>(null)

  function handleSubmit(event: Event): void {
    event.preventDefault()
    const token = value.trim()
    if (!token) {
      setError('请输入访问令牌')
      return
    }
    storeToken(token)
    onValid(token)
  }

  return (
    <div class="token-gate">
      <form class="card token-card" onSubmit={handleSubmit}>
        <div class="gate-logo">
          <LogoMark size={76} />
        </div>
        <div>
          <h1>Just Print</h1>
          <p class="muted">请输入访问令牌以使用打印服务</p>
        </div>
        <label class="gate-input" aria-label="访问令牌">
          <KeyIcon size={17} />
          <input
            type="password"
            value={value}
            placeholder="JUST_PRINT_TOKEN"
            autocomplete="off"
            onInput={(event) => {
              setValue((event.target as HTMLInputElement).value)
              setError(null)
            }}
          />
        </label>
        <button type="submit" class="primary" disabled={value.trim().length === 0}>
          <LockIcon size={16} />
          进入
        </button>
        {error ? (
          <p class="error">
            <AlertIcon size={15} />
            {error}
          </p>
        ) : null}
      </form>
    </div>
  )
}
