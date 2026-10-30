import { afterEach, expect, it, vi } from 'vitest'
import type { FileTransfer } from '../src/shared/transfers'
const bridge = vi.hoisted(() => ({
  invoke: vi.fn(),
  mounted: [] as (() => void)[],
  unmounted: [] as (() => void)[],
}))
vi.mock('@tauri-apps/api/core', () => ({ invoke: bridge.invoke, isTauri: () => true }))
vi.mock('vue', async (original) => ({
  ...(await original<typeof import('vue')>()),
  onMounted: (f: () => void) => bridge.mounted.push(f),
  onUnmounted: (f: () => void) => bridge.unmounted.push(f),
}))
import { useTransfers } from '../src/composables/useTransfers'
function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((r) => {
    resolve = r
  })
  return { promise, resolve }
}
afterEach(() => {
  bridge.unmounted.forEach((f) => f())
  bridge.mounted.length = bridge.unmounted.length = 0
  vi.resetAllMocks()
})
it('does not let a slow polling response overwrite a newer post-action snapshot', async () => {
  const old = deferred<{ transfers: FileTransfer[]; warnings: string[] }>()
  const latest = [{ id: 'job', status: 'complete', completedBytes: 12 }] as FileTransfer[]
  bridge.invoke.mockImplementation((command: string) => {
    if (command === 'file_action') return Promise.resolve()
    if (bridge.invoke.mock.calls.filter(([name]) => name === 'file_snapshot').length === 1)
      return old.promise
    return Promise.resolve({ transfers: latest, warnings: ['record retained'] })
  })
  const files = useTransfers()
  bridge.mounted[0]!()
  await files.act({ type: 'pause', id: 'job' })
  expect(files.transfers.value).toEqual(latest)
  old.resolve({
    transfers: [{ id: 'job', status: 'transferring' }] as FileTransfer[],
    warnings: [],
  })
  await Promise.resolve()
  await Promise.resolve()
  expect(files.transfers.value).toEqual(latest)
  expect(files.warnings.value).toEqual(['record retained'])
})
it('clears busy state and presents a native error after an unsuccessful action', async () => {
  bridge.invoke.mockRejectedValue('接收磁盘空间不足')
  const files = useTransfers()
  await files.act({ type: 'accept', id: 'job' })
  expect(files.busy.value).toBe(false)
  expect(files.error.value).toContain('空间不足')
})
it('discards a snapshot that arrives after the component is disposed', async () => {
  const response = deferred<{ transfers: FileTransfer[]; warnings: string[] }>()
  bridge.invoke.mockReturnValue(response.promise)
  const files = useTransfers()
  bridge.mounted[0]!()
  bridge.unmounted[0]!()
  response.resolve({ transfers: [{ id: 'late' }] as FileTransfer[], warnings: [] })
  await Promise.resolve()
  await Promise.resolve()
  expect(files.transfers.value).toEqual([])
})
it('sends each explicit conflict decision with its task and prompt token', async () => {
  bridge.invoke.mockImplementation((command: string) =>
    Promise.resolve(command === 'file_snapshot' ? { transfers: [], warnings: [] } : undefined),
  )
  const files = useTransfers()
  for (const decision of ['replace', 'keepBoth', 'stop'] as const) {
    const action = {
      type: 'resolveConflict',
      id: 'job',
      token: 'current-choice',
      decision,
    } as const
    await files.act(action)
    expect(bridge.invoke).toHaveBeenCalledWith('file_action', { action })
    expect(files.error.value).toBe('')
  }
})
