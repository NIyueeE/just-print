import { useState } from 'preact/hooks'
import { storeToken } from '../api'

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
        <h1>Just Print</h1>
        <p class="muted">请输入访问令牌以使用打印服务</p>
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
        <button type="submit" disabled={value.trim().length === 0}>
          进入
        </button>
        {error ? <p class="error">{error}</p> : null}
      </form>
    </div>
  )
}
