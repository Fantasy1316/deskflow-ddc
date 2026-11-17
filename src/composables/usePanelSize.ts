import { onMounted, onUnmounted } from 'vue'

// Observe natural content height; the native window stays anchored to the tray.
export function usePanelSize(
  element: () => HTMLElement | null,
  resize: (height: number) => unknown,
) {
  let observer: ResizeObserver | undefined
  let frame = 0
  let previous = 0
  onMounted(() => {
    const node = element()
    if (!node || typeof ResizeObserver === 'undefined') return
    observer = new ResizeObserver(() => {
      cancelAnimationFrame(frame)
      frame = requestAnimationFrame(() => {
        const height = Math.ceil(node.getBoundingClientRect().height)
        if (height !== previous) {
          previous = height
          void resize(height)
        }
      })
    })
    observer.observe(node)
  })
  onUnmounted(() => {
    observer?.disconnect()
    cancelAnimationFrame(frame)
  })
}
