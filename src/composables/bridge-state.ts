import type { BridgeAPI, Snapshot } from '../shared/types'

/** Subscribe without letting a delayed bootstrap snapshot overwrite a push. */
export function initializeBridge(
  api: Pick<BridgeAPI, 'subscribe' | 'snapshot'>,
  apply: (state: Snapshot) => void,
) {
  let disposed = false,
    pushed = false
  const unsubscribe = api.subscribe((state) => {
    if (disposed) return
    pushed = true
    apply(state)
  })
  const ready = api.snapshot().then((state) => {
    if (!disposed && !pushed) apply(state)
  })
  return {
    ready,
    dispose: () => {
      disposed = true
      unsubscribe()
    },
  }
}
