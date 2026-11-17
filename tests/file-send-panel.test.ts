import { afterEach, expect, it, vi } from 'vitest'
const bridge = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
  mounted: [] as (() => Promise<void>)[],
  unmounted: [] as (() => void)[],
}))
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true, invoke: bridge.invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: bridge.listen }))
vi.mock('vue', async (original) => ({
  ...(await original<typeof import('vue')>()),
  onMounted: (f: () => Promise<void>) => bridge.mounted.push(f),
  onUnmounted: (f: () => void) => bridge.unmounted.push(f),
}))
import { useFileSendPanel, type FileSendView } from '../src/composables/useFileSendPanel'

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: unknown) => void
  const promise = new Promise<T>((done, fail) => {
    resolve = done
    reject = fail
  })
  return { promise, resolve, reject }
}
const ready: FileSendView = {
  revision: 1,
  peerName: 'Windows',
  peerId: 'intended-peer',
  disabledReason: null,
  busy: false,
  message: null,
  error: false,
  dragging: false,
  scannedEntries: null,
  activity: null,
}
function fixture() {
  const stop = vi.fn()
  let receive!: (event: { payload: FileSendView }) => void
  bridge.listen.mockImplementation((_event, callback) => {
    receive = callback
    return Promise.resolve(stop)
  })
  bridge.invoke.mockResolvedValue(ready)
  const panel = useFileSendPanel()
  return {
    panel,
    stop,
    event: (value: Partial<FileSendView>) => receive({ payload: { ...ready, ...value } }),
  }
}
afterEach(() => {
  bridge.unmounted.forEach((f) => f())
  bridge.mounted.length = bridge.unmounted.length = 0
  vi.resetAllMocks()
})
it('subscribes before the snapshot and ignores a snapshot older than an event', async () => {
  const f = fixture(),
    snapshot = deferred<FileSendView>()
  bridge.invoke.mockReturnValue(snapshot.promise)
  const mounting = bridge.mounted[0]!()
  expect(bridge.invoke).not.toHaveBeenCalled()
  await Promise.resolve()
  expect(bridge.listen).toHaveBeenCalledWith('file-send-state', expect.any(Function), {
    target: 'file-send',
  })
  f.event({ revision: 5, peerName: 'New peer' })
  snapshot.resolve(ready)
  await mounting
  expect(f.panel.state.value.peerName).toBe('New peer')
  expect(bridge.invoke).toHaveBeenCalledExactlyOnceWith('file_send_snapshot')
})
it('waits for an explicit picker request, binds it to the visible peer and coalesces clicks', async () => {
  const f = fixture()
  await bridge.mounted[0]!()
  expect(bridge.invoke).toHaveBeenCalledTimes(1)
  const picking = deferred<void>()
  bridge.invoke.mockImplementation((_command, args) =>
    args?.action.type === 'chooseFiles' ? picking.promise : Promise.resolve(),
  )
  f.panel.choose()
  f.panel.choose(true)
  expect(f.panel.pending.value).toBe(true)
  expect(bridge.invoke).toHaveBeenLastCalledWith('file_send_action', {
    action: { type: 'chooseFiles', peerId: 'intended-peer' },
  })
  await f.panel.close()
  await f.panel.showMain()
  // Closing/opening the main window must not clear an in-flight picker lock.
  expect(f.panel.pending.value).toBe(true)
  f.event({ revision: 2, peerId: 'new-peer', peerName: 'Other computer' })
  expect(
    bridge.invoke.mock.calls.filter(([, args]) => args?.action.type === 'chooseFiles'),
  ).toHaveLength(1)
  picking.resolve()
  await picking.promise
  expect(f.panel.pending.value).toBe(false)
  f.panel.choose(true)
  expect(bridge.invoke).toHaveBeenLastCalledWith('file_send_action', {
    action: { type: 'chooseFolder', peerId: 'new-peer' },
  })
})
it.each([{ disabledReason: '请先连接电脑' }, { busy: true }, { peerId: null }])(
  'does not start a picker when sending is unavailable: %j',
  async (value) => {
    const f = fixture()
    await bridge.mounted[0]!()
    f.event({ ...value, revision: 2 })
    bridge.invoke.mockClear()
    f.panel.choose()
    f.panel.choose(true)
    expect(bridge.invoke).not.toHaveBeenCalled()
    await f.panel.showMain()
    expect(bridge.invoke).toHaveBeenCalledExactlyOnceWith('file_send_action', {
      action: { type: 'showMain' },
    })
  },
)
it('shows a picker error and clears it when a newer native send updates the panel', async () => {
  const f = fixture()
  await bridge.mounted[0]!()
  const picking = deferred<void>()
  bridge.invoke.mockReturnValue(picking.promise)
  f.panel.choose()
  picking.reject('连接的电脑已更换')
  await picking.promise.catch(() => {})
  expect(f.panel.error.value).toContain('已更换')
  expect(f.panel.pending.value).toBe(false)
  f.event({ revision: 2, message: '已提交 1 项' })
  expect(f.panel.error.value).toBe('')
})
it('unsubscribes and ignores late state or commands after disposal', async () => {
  const f = fixture()
  await bridge.mounted[0]!()
  bridge.unmounted[0]!()
  expect(f.stop).toHaveBeenCalledOnce()
  f.event({ revision: 10, peerName: 'late' })
  expect(f.panel.state.value.peerName).toBe('Windows')
  bridge.invoke.mockClear()
  f.panel.choose()
  await f.panel.close()
  expect(bridge.invoke).not.toHaveBeenCalled()
})
it('removes a listener whose registration finishes after disposal', async () => {
  const f = fixture(),
    registration = deferred<() => void>()
  bridge.listen.mockReturnValue(registration.promise)
  const mounting = bridge.mounted[0]!()
  bridge.unmounted[0]!()
  registration.resolve(f.stop)
  await mounting
  expect(f.stop).toHaveBeenCalledOnce()
  expect(bridge.invoke).not.toHaveBeenCalled()
})

it('does not display an old recipient picker failure after pairing changes', async () => {
  const f = fixture()
  await bridge.mounted[0]!()
  const picking = deferred<void>()
  bridge.invoke.mockReturnValue(picking.promise)
  f.panel.choose()
  f.event({ revision: 2, peerId: 'new-peer', peerName: 'Other computer' })
  picking.reject('旧电脑已断开')
  await picking.promise.catch(() => {})
  expect(f.panel.error.value).toBe('')
  expect(f.panel.pending.value).toBe(false)
})

it('retries only the visible paused task for its current recipient and resizes without a send lock', async () => {
  const f = fixture()
  await bridge.mounted[0]!()
  bridge.invoke.mockResolvedValue(undefined)
  f.panel.retry()
  expect(bridge.invoke).toHaveBeenCalledTimes(1)
  f.event({
    revision: 2,
    activity: {
      id: 'paused-task',
      label: 'file.zip',
      status: 'paused',
      items: 1,
      completedBytes: 0,
      totalBytes: 10,
      bytesPerSecond: 0,
      error: '断线',
    },
  })
  f.panel.retry()
  expect(bridge.invoke).toHaveBeenLastCalledWith('file_send_action', {
    action: { type: 'retry', id: 'paused-task', peerId: 'intended-peer' },
  })
  await f.panel.resize(900)
  expect(bridge.invoke).toHaveBeenLastCalledWith('file_send_action', {
    action: { type: 'resize', height: 480 },
  })
  await f.panel.resize(10)
  expect(bridge.invoke).toHaveBeenLastCalledWith('file_send_action', {
    action: { type: 'resize', height: 260 },
  })
})
