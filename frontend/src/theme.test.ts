import { describe, expect, it } from 'vitest'
import { nextThemePreference, resolveTheme, type ThemePreference } from './theme'

describe('主题循环', () => {
  it('cycles system → light → dark → system', () => {
    // 从跟随系统出发：切到与当前显示相反的一侧。
    expect(nextThemePreference('system', 'dark')).toBe('light')
    expect(nextThemePreference('system', 'light')).toBe('dark')

    expect(nextThemePreference('light', 'light')).toBe('dark')
    // 必须能回到跟随系统，否则手动切换一次就再也回不去了。
    expect(nextThemePreference('dark', 'dark')).toBe('system')
  })

  it('reaches the system preference again from any manual preference', () => {
    for (const start of ['light', 'dark'] as const) {
      let preference: ThemePreference = start
      for (let step = 0; step < 3 && preference !== 'system'; step += 1) {
        preference = nextThemePreference(preference, 'light')
      }
      expect(preference).toBe('system')
    }
  })

  it('resolves the system preference from the media query result', () => {
    expect(resolveTheme('system', true)).toBe('dark')
    expect(resolveTheme('system', false)).toBe('light')
    expect(resolveTheme('light', true)).toBe('light')
    expect(resolveTheme('dark', false)).toBe('dark')
  })
})
