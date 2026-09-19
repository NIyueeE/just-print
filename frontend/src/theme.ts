/* 主题偏好：system（默认）/ light / dark，手动覆盖写入 localStorage。 */

export type ThemePreference = 'system' | 'light' | 'dark'
export type ResolvedTheme = 'light' | 'dark'

export const THEME_STORAGE_KEY = 'just_print_theme'

export function isThemePreference(value: unknown): value is ThemePreference {
  return value === 'system' || value === 'light' || value === 'dark'
}

export function readThemePreference(): ThemePreference {
  try {
    const stored = localStorage.getItem(THEME_STORAGE_KEY)
    return isThemePreference(stored) ? stored : 'system'
  } catch {
    return 'system'
  }
}

export function resolveTheme(
  preference: ThemePreference,
  prefersDark: boolean = typeof window !== 'undefined' && typeof window.matchMedia === 'function'
    ? window.matchMedia('(prefers-color-scheme: dark)').matches
    : true,
): ResolvedTheme {
  if (preference === 'system') {
    return prefersDark ? 'dark' : 'light'
  }
  return preference
}

/** 把偏好写入 `<html data-theme>`；system 时移除属性交给媒体查询。 */
export function applyThemePreference(preference: ThemePreference): void {
  const root = document.documentElement
  if (preference === 'system') {
    root.removeAttribute('data-theme')
  } else {
    root.setAttribute('data-theme', preference)
  }
  try {
    if (preference === 'system') {
      localStorage.removeItem(THEME_STORAGE_KEY)
    } else {
      localStorage.setItem(THEME_STORAGE_KEY, preference)
    }
  } catch {
    /* 隐私模式下 localStorage 不可写，忽略即可。 */
  }
}

/** 在“跟随系统 → 明 → 暗”之间循环。 */
export function nextThemePreference(
  preference: ThemePreference,
  resolved: ResolvedTheme,
): ThemePreference {
  if (preference === 'system') {
    return resolved === 'dark' ? 'light' : 'dark'
  }
  return preference === 'dark' ? 'light' : 'dark'
}
