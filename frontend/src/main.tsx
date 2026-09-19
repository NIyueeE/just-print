import { render } from 'preact'
import './styles/tokens.css'
import './styles/base.css'
import { App } from './app'
import { AppStateProvider } from './state'
import { applyThemePreference, readThemePreference } from './theme'

// 在首帧前应用主题偏好（CSP 不允许内联脚本，因此放在入口模块里）。
applyThemePreference(readThemePreference())

const container = document.getElementById('app')
if (container !== null) {
  render(
    <AppStateProvider>
      <App />
    </AppStateProvider>,
    container,
  )
}
