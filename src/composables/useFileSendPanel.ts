import { computed, onMounted, onUnmounted, readonly, shallowRef } from 'vue'
import { invoke, isTauri } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type { TransferStatus } from '../shared/transfers'

export interface FileSendActivity {
  id: string
  label: string
  status: TransferStatus
  items: number
  totalBytes: number
  completedBytes: number
  bytesPerSecond: number
  error: string | null
}

export interface FileSendView {
  revision: number
  peerName: string
  peerId: string | null
  disabledReason: string | null
  busy: boolean
  message: string | null
  error: boolean
  dragging: boolean
  scannedEntries: number | null
  activity: FileSendActivity | null
}
type SendAction =
  | { type: 'chooseFiles' | 'chooseFolder'; peerId: string }
  | { type: 'showMain' | 'close' }
  | { type: 'retry'; id: string; peerId: string }
  | { type: 'resize'; height: number }

export function useFileSendPanel() {
  const preview = !isTauri()
  const state = shallowRef<FileSendView>({
    revision: -1,
    peerName: '',
    peerId: null,
    disabledReason: preview ? '浏览器预览，发送文件请使用桌面 App' : '正在读取连接状态…',
    busy: false,
    message: null,
    error: false,
    dragging: false,
    scannedEntries: null,
    activity: null,
  })
  const pending = shallowRef(false)
  const error = shallowRef('')
  const disabled = computed(
    () => pending.value || state.value.busy || !!state.value.disabledReason || !state.value.peerId,
  )
  let disposed = false
  let unlisten: (() => void) | undefined
  function apply(value: FileSendView) {
    if (!disposed && value.revision > state.value.revision) {
      state.value = value
      error.value = ''
    }
  }
  async function act(action: SendAction) {
    if (preview || disposed) return
    const sends = ['chooseFiles', 'chooseFolder', 'retry'].includes(action.type)
    if (sends && disabled.value) return
    if (sends) {
      pending.value = true
      error.value = ''
    }
    try {
      await invoke('file_send_action', { action })
    } catch (e) {
      if (
        !disposed &&
        action.type !== 'resize' &&
        (!('peerId' in action) || action.peerId === state.value.peerId)
      )
        error.value = String(e)
    } finally {
      if (sends && !disposed) pending.value = false
    }
  }
  function choose(folder = false) {
    if (state.value.peerId)
      void act({ type: folder ? 'chooseFolder' : 'chooseFiles', peerId: state.value.peerId })
  }
  onMounted(async () => {
    if (preview) return
    try {
      const stop = await listen<FileSendView>('file-send-state', ({ payload }) => apply(payload), {
        target: 'file-send',
      })
      if (disposed) {
        stop()
        return
      }
      unlisten = stop
      apply(await invoke<FileSendView>('file_send_snapshot'))
    } catch (e) {
      if (!disposed) error.value = `无法读取发送面板状态：${String(e)}`
    }
  })
  onUnmounted(() => {
    disposed = true
    unlisten?.()
  })
  return {
    state: readonly(state),
    pending: readonly(pending),
    error: readonly(error),
    disabled,
    choose,
    close: () => act({ type: 'close' }),
    showMain: () => act({ type: 'showMain' }),
    retry: () => {
      const { activity, peerId } = state.value
      if (activity?.status === 'paused' && peerId)
        void act({ type: 'retry', id: activity.id, peerId })
    },
    resize: (height: number) =>
      act({ type: 'resize', height: Math.max(260, Math.min(480, Math.ceil(height))) }),
  }
}
