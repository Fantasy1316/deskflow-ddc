import { onMounted, onUnmounted, shallowRef } from 'vue'
import { invoke, isTauri } from '@tauri-apps/api/core'
import type { FileAction, FileTransfer, FileTransferNetwork } from '../shared/transfers'

export function useTransfers() {
  const transfers = shallowRef<FileTransfer[]>([])
  const network = shallowRef<FileTransferNetwork | null>(null)
  const busy = shallowRef(false)
  const error = shallowRef('')
  const warnings = shallowRef<string[]>([])
  let generation = 0
  const speeds = shallowRef<Record<string, number>>({})
  let timer: ReturnType<typeof setTimeout> | undefined,
    disposed = false
  let samples = new Map<string, { bytes: number; time: number }>()
  async function refresh() {
    if (!isTauri() || disposed) return
    const request = ++generation
    const snapshot = await invoke<{
      transfers: FileTransfer[]
      warnings: string[]
      network?: FileTransferNetwork
    }>('file_snapshot')
    if (disposed || request !== generation) return
    warnings.value = snapshot.warnings
    network.value = snapshot.network ?? null
    const next = snapshot.transfers
    const now = performance.now(),
      rates: Record<string, number> = {},
      updated = new Map<string, { bytes: number; time: number }>()
    for (const job of next) {
      const sample = samples.get(job.id)
      rates[job.id] =
        job.status === 'transferring' && sample
          ? Math.max(0, ((job.completedBytes - sample.bytes) * 1000) / (now - sample.time))
          : 0
      updated.set(job.id, { bytes: job.completedBytes, time: now })
    }
    samples = updated
    speeds.value = rates
    transfers.value = next
  }
  async function poll() {
    try {
      await refresh()
    } catch (e) {
      if (!disposed) error.value = String(e)
    } finally {
      if (!disposed) timer = setTimeout(poll, 1000)
    }
  }
  async function act(action: FileAction) {
    if (busy.value || !isTauri()) return
    busy.value = true
    error.value = ''
    try {
      await invoke('file_action', { action })
      await refresh()
    } catch (e) {
      if (!disposed) error.value = String(e)
    } finally {
      busy.value = false
    }
  }
  onMounted(() => {
    void poll()
  })
  onUnmounted(() => {
    disposed = true
    clearTimeout(timer)
    samples.clear()
  })
  return { transfers, busy, error, warnings, speeds, network, act }
}
