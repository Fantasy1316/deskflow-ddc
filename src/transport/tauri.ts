import { invoke, isTauri } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type { BridgeAPI, Snapshot } from '../shared/types'
import { actionSchema } from '../shared/actions'

/** Register the native event listener before any bootstrap snapshot is requested. */
export async function installTauriBridge(): Promise<void> {
  if (!isTauri() || window.screenbridge) return
  const listeners = new Set<(state: Snapshot) => void>()
  let revision = -1
  const unlisten = await listen<Snapshot>('bridge-state', ({ payload }) => {
    if ((payload.revision ?? 0) <= revision) return
    revision = payload.revision ?? 0
    for (const listener of listeners) listener(payload)
  })
  const bridge: BridgeAPI = {
    snapshot: () => invoke<Snapshot>('snapshot'),
    act: (action) => invoke('act', { action: actionSchema.parse(action) }),
    reportError: (error) => invoke('report_error', { error }),
    subscribe: (listener) => {
      listeners.add(listener)
      return () => listeners.delete(listener)
    },
  }
  window.screenbridge = bridge
  window.addEventListener(
    'beforeunload',
    () => {
      unlisten()
      listeners.clear()
    },
    { once: true },
  )
}
