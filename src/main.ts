import { createApp } from 'vue'
import App from './App.vue'
import './style.css'
import type { RendererError } from './shared/types'
function reportError(error: unknown, source: RendererError['source']) {
  const detail =
    error instanceof Error
      ? { message: error.message.slice(0, 8192), stack: error.stack?.slice(0, 16384) }
      : { message: '发生非 Error 类型的界面异常（未收集原始内容）' }
  void window.screenbridge?.reportError({ source, ...detail }).catch(() => {
    /* Avoid recursive reporting. */
  })
}
window.addEventListener('error', (event) =>
  reportError(event.error ?? new Error(event.message), 'window.error'),
)
window.addEventListener('unhandledrejection', (event) =>
  reportError(event.reason, 'window.unhandledrejection'),
)
const app = createApp(App)
app.config.errorHandler = (error) => reportError(error, 'vue')
app.mount('#app')
