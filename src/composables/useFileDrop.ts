import { onMounted, onUnmounted, shallowRef } from 'vue'
import { isTauri } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

interface FileDrag {
  kind: 'enter' | 'over' | 'leave' | 'drop'
  position?: { x: number; y: number }
  token?: string
  error?: string
}
export function useFileDrop(
  element: () => HTMLElement | null,
  disabled: () => boolean,
  send: (token: string) => void,
) {
  const dragging = shallowRef(false)
  const error = shallowRef('')
  let disposed = false
  let unlisten: (() => void) | undefined
  onMounted(async () => {
    if (!isTauri()) return
    try {
      const stop = await listen<FileDrag>(
        'file-drag',
        ({ payload }) => {
          if (disposed) return
          if (payload.kind === 'leave' || disabled()) {
            dragging.value = false
            return
          }
          const rect = element()?.getBoundingClientRect()
          // Native coordinates are normalized to logical window coordinates.
          const x = payload.position?.x ?? NaN
          const y = payload.position?.y ?? NaN
          const inside =
            !!rect && x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
          dragging.value = inside && payload.kind !== 'drop'
          if (payload.kind !== 'drop' || !inside) return
          error.value = payload.error ?? ''
          if (payload.token && !payload.error) send(payload.token)
        },
        { target: 'main' },
      )
      if (disposed) stop()
      else unlisten = stop
    } catch (e) {
      if (!disposed) error.value = `无法启用拖放，可点击选择文件：${String(e)}`
    }
  })
  onUnmounted(() => {
    disposed = true
    dragging.value = false
    unlisten?.()
  })
  return { dragging, error }
}
