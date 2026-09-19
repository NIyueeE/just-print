import { useEffect, useState } from 'preact/hooks'
import { MoonIcon, SunIcon } from '../icons'
import {
  applyThemePreference,
  nextThemePreference,
  readThemePreference,
  resolveTheme,
  type ResolvedTheme,
  type ThemePreference,
} from '../theme'
import './ThemeToggle.css'

/** 手动主题切换：跟随系统 → 明 → 暗，偏好持久化在 localStorage。 */
export function ThemeToggle() {
  const [preference, setPreference] = useState<ThemePreference>(() => readThemePreference())
  const [resolved, setResolved] = useState<ResolvedTheme>(() => resolveTheme(readThemePreference()))

  useEffect(() => {
    applyThemePreference(preference)
    setResolved(resolveTheme(preference))
    if (preference !== 'system' || typeof window.matchMedia !== 'function') {
      return
    }
    const media = window.matchMedia('(prefers-color-scheme: dark)')
    const onChange = (): void => setResolved(resolveTheme('system', media.matches))
    media.addEventListener('change', onChange)
    return () => media.removeEventListener('change', onChange)
  }, [preference])

  const nextLabel = resolved === 'dark' ? '切换到浅色主题' : '切换到深色主题'

  return (
    <button
      type="button"
      class="ghost theme-toggle"
      aria-label={nextLabel}
      title={nextLabel}
      onClick={() =>
        setPreference((current) => nextThemePreference(current, resolveTheme(current)))
      }
    >
      {resolved === 'dark' ? <SunIcon size={16} /> : <MoonIcon size={16} />}
      <span class="theme-toggle__label">{resolved === 'dark' ? '浅色' : '深色'}</span>
    </button>
  )
}
