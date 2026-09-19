import { useEffect, useState } from 'preact/hooks'
import { MonitorIcon, MoonIcon, SunIcon } from '../icons'
import {
  applyThemePreference,
  nextThemePreference,
  readThemePreference,
  resolveTheme,
  type ResolvedTheme,
  type ThemePreference,
} from '../theme'
import './ThemeToggle.css'

const PREFERENCE_LABEL: Record<ThemePreference, string> = {
  system: '跟随系统',
  light: '浅色',
  dark: '深色',
}

/** 手动主题切换：跟随系统 → 明 → 暗 → 跟随系统，偏好持久化在 localStorage。 */
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

  // 图标与文案表示「当前」偏好；可访问名说明「点击之后会怎样」。
  const next = nextThemePreference(preference, resolveTheme(preference))
  const actionLabel = next === 'system' ? '恢复跟随系统主题' : `切换到${PREFERENCE_LABEL[next]}主题`

  return (
    <button
      type="button"
      class="ghost theme-toggle"
      aria-label={actionLabel}
      title={`${actionLabel}（当前：${PREFERENCE_LABEL[preference]}）`}
      onClick={() =>
        setPreference((current) => nextThemePreference(current, resolveTheme(current)))
      }
    >
      {preference === 'system' ? (
        <MonitorIcon size={16} />
      ) : resolved === 'dark' ? (
        <MoonIcon size={16} />
      ) : (
        <SunIcon size={16} />
      )}
      <span class="theme-toggle__label">{PREFERENCE_LABEL[preference]}</span>
    </button>
  )
}
