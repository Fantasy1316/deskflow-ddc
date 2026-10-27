import { computed, onMounted, onUnmounted, shallowRef } from 'vue'
import type { Action, Snapshot } from '../shared/types'
import { defaultConfig } from '../shared/config'
import { initializeBridge } from './bridge-state'

function previewSnapshot(): Snapshot {
  return {
    cursorDisplays: [],
    sessionLayout: null,
    config: defaultConfig('我的电脑'),
    deviceId: 'preview',
    platform: 'preview',
    addresses: [],
    monitors: [],
    running: false,
    listening: false,
    connected: false,
    engine: 'stopped',
    peer: null,
    activeDeviceId: null,
    displayDeviceId: null,
    switching: false,
    pairingCode: '',
    dependencies: { deskflow: false, displayControl: false, accessibility: false },
    logs: [],
    lastError: null,
    preview: true,
    diagnostics: { directory: '', error: null },
  }
}
export function useBridge() {
  const state = shallowRef<Snapshot>(previewSnapshot())
  const loading = shallowRef(true),
    busy = shallowRef(false),
    pendingAction = shallowRef<Action['type'] | null>(null),
    notice = shallowRef(''),
    noticeError = shallowRef(false)
  let unsubscribe: (() => void) | undefined, timer: ReturnType<typeof setTimeout> | undefined
  function notify(message: string, error = false) {
    clearTimeout(timer)
    notice.value = message
    noticeError.value = error
    timer = setTimeout(() => {
      notice.value = ''
    }, 7000)
  }
  async function act(action: Action) {
    if (busy.value) return false
    busy.value = true
    pendingAction.value = action.type
    try {
      if (!window.screenbridge) {
        if (action.type === 'save') {
          state.value = { ...state.value, config: structuredClone(action.config) }
          notify('已更新预览配置。硬件功能请在桌面 App 中使用。')
        } else notify('当前为浏览器预览，请打开桌面 App 执行此操作。', true)
      } else {
        await window.screenbridge.act(action)
        if (action.type === 'save') notify('设置已保存')
        if (action.type === 'copyPairing') notify('配对码已复制')
        if (action.type === 'checkAccessibility') notify('权限检测已完成')
      }
      return true
    } catch (error) {
      notify(
        String(error instanceof Error ? error.message : error).replace(
          /^Error invoking remote method '[^']+': Error: /,
          '',
        ),
        true,
      )
      return false
    } finally {
      busy.value = false
      pendingAction.value = null
    }
  }
  const displayConfigured = computed(
    () =>
      !!state.value.config.displayId &&
      state.value.config.localInput !== null &&
      state.value.config.peerInput !== null,
  )
  onMounted(async () => {
    try {
      if (window.screenbridge) {
        const connection = initializeBridge(window.screenbridge, (value) => {
          state.value = value
        })
        unsubscribe = connection.dispose
        await connection.ready
      }
    } catch (error) {
      notify(String(error), true)
    } finally {
      loading.value = false
    }
  })
  onUnmounted(() => {
    unsubscribe?.()
    clearTimeout(timer)
  })
  return { state, loading, busy, pendingAction, notice, noticeError, displayConfigured, act }
}
