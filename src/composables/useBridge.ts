import { computed, onMounted, onUnmounted, shallowRef, watch } from 'vue'
import type { Action, Config, Snapshot } from '../shared/types'
import { defaultConfig } from '../shared/config'
import { initializeBridge } from './bridge-state'
import { displayMappingValid, saveThenStart, type ActionFailure } from '../shared/workspace'

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
    hasSavedPairing: false,
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
  const actionFailure = shallowRef<ActionFailure | null>(null)
  const dismissedRuntimeError = shallowRef<string | null>(null)
  const failure = computed<ActionFailure | null>(
    () =>
      actionFailure.value ??
      (state.value.lastError && state.value.lastError !== dismissedRuntimeError.value
        ? { action: 'runtime', message: state.value.lastError }
        : null),
  )
  watch(
    () => state.value.lastError,
    () => {
      dismissedRuntimeError.value = null
    },
  )
  function dismissFailure() {
    actionFailure.value = null
    dismissedRuntimeError.value = state.value.lastError
  }
  let unsubscribe: (() => void) | undefined, timer: ReturnType<typeof setTimeout> | undefined
  function notify(message: string, error = false) {
    clearTimeout(timer)
    notice.value = message
    noticeError.value = error
    timer = setTimeout(() => {
      notice.value = ''
    }, 7000)
  }
  async function perform(action: Action) {
    pendingAction.value = action.type
    try {
      if (!window.screenbridge) {
        if (action.type === 'save') {
          state.value = { ...state.value, config: structuredClone(action.config) }
          notify('已更新预览配置。硬件功能请在桌面 App 中使用。')
        } else throw new Error('当前为浏览器预览，请打开桌面 App 执行此操作。')
      } else {
        await window.screenbridge.act(action)
        if (action.type === 'save') notify('设置已保存')
        if (action.type === 'copyPairing') notify('配对码已复制')
        if (action.type === 'checkAccessibility') notify('权限检测已完成')
        if (action.type === 'installWindowsService')
          notify('后台服务已安装，请启用安全桌面控制并保存设置')
        if (action.type === 'removeWindowsService') notify('后台服务已移除，已恢复普通共享模式')
        if (action.type === 'checkWindowsService') notify('后台服务检测已完成')
        if (action.type === 'lockPeer') notify('Windows 已接受锁屏请求')
      }
      if (actionFailure.value?.action === action.type) actionFailure.value = null
      return true
    } catch (error) {
      const message = String(error instanceof Error ? error.message : error).replace(
        /^Error invoking remote method '[^']+': Error: /,
        '',
      )
      actionFailure.value = {
        action: action.type,
        message: message || '未收到错误详情，请导出诊断日志。',
      }
      notify(actionFailure.value.message, true)
      return false
    }
  }
  async function runExclusive(operation: () => Promise<boolean>) {
    if (busy.value) return false
    busy.value = true
    try {
      return await operation()
    } finally {
      busy.value = false
      pendingAction.value = null
    }
  }
  const act = (action: Action) => runExclusive(() => perform(action))
  const saveAndStart = (config: Config, pairingCode?: string) =>
    runExclusive(() => saveThenStart(perform, config, pairingCode))
  const displayConfigured = computed(() => displayMappingValid(state.value.config))
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
      actionFailure.value = { action: 'snapshot', message: String(error) }
      notify(String(error), true)
    } finally {
      loading.value = false
    }
  })
  onUnmounted(() => {
    unsubscribe?.()
    clearTimeout(timer)
  })
  return {
    state,
    loading,
    busy,
    pendingAction,
    notice,
    noticeError,
    displayConfigured,
    failure,
    dismissFailure,
    act,
    saveAndStart,
  }
}
