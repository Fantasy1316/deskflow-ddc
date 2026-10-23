import { afterEach, describe, expect, it, vi } from 'vitest'
import { EventEmitter } from 'node:events'
import { createServer } from 'node:net'
import { randomUUID } from 'node:crypto'
import { mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { Runtime } from '../electron/core/runtime'
import { defaults } from '../electron/core/config'
import { newPairingCode } from '../electron/core/channel'
import type { Store } from '../electron/core/store'
import type { Peer } from '../src/shared/types'
import { DiagnosticLog, diagnosticSnapshot } from '../electron/core/diagnostics'

const mocks = vi.hoisted(() => ({
  engines: [] as EventEmitter[],
  starts: [] as Peer[],
  writes: [] as Array<{ machine: string; input: number }>,
  injections: [] as string[],
}))
vi.mock('../electron/core/engine', async () => {
  const { EventEmitter } = await import('node:events')
  return {
    findDeskflow: async () => '/mock/deskflow-core',
    DeskflowEngine: class extends EventEmitter {
      constructor() {
        super()
        mocks.engines.push(this)
      }
      async identity() {
        return { fingerprint: 'a'.repeat(64) }
      }
      async assertAvailable() {}
      async start(_config: unknown, _localId: string, peer: Peer) {
        mocks.starts.push({ ...peer, id: peer.id })
        this.emit('state', 'ready')
      }
      async stop() {
        this.emit('state', 'stopped')
      }
    },
  }
})
vi.mock('../electron/platform/display', () => ({
  DisplayController: class {
    constructor(private machine: string) {}
    async available() {
      return true
    }
    async checkAccessibility() {
      return true
    }
    async setInput(_id: string, input: number) {
      mocks.writes.push({ machine: this.machine, input })
    }
    async injectTarget(target: string) {
      mocks.injections.push(target)
    }
  },
}))
const running: Runtime[] = []
afterEach(async () => {
  await Promise.all(running.splice(0).map((r) => r.stop()))
  mocks.engines.length = 0
  mocks.starts.length = 0
  mocks.writes.length = 0
  mocks.injections.length = 0
})
async function unusedPort() {
  const server = createServer()
  await new Promise<void>((resolve, reject) => {
    server.once('error', reject)
    server.listen(0, '0.0.0.0', () => {
      server.removeListener('error', reject)
      resolve()
    })
  })
  const port = (server.address() as { port: number }).port
  await new Promise<void>((resolve) => server.close(() => resolve()))
  return port
}
async function waitFor(test: () => boolean) {
  await vi.waitFor(() => expect(test()).toBe(true), { timeout: 2500, interval: 10 })
}
async function createPair(
  reverse = false,
  saveIdentity = async (_role: 'host' | 'client') => {},
  diagnostics?: DiagnosticLog,
) {
  const port = await unusedPort(),
    code = newPairingCode(),
    ids = [randomUUID(), randomUUID()]
  const make = async (role: 'host' | 'client', index: number) => {
    const local = (index === 0) !== reverse ? 3 : 1
    const config = {
      ...defaults(index === 0 ? 'Mac' : 'Windows'),
      role,
      localInput: local,
      peerInput: local === 1 ? 3 : 1,
      displayId: 'display',
      peerAddress: role === 'client' ? '127.0.0.1' : '',
      controlPort: port,
      deskflowPort: port === 24800 ? 24801 : 24800,
    }
    const store = {
      dir: '/mock/' + index,
      value: { id: ids[index], config, pairingCode: code, trustedPeer: null },
      async load() {},
      async save() {
        await saveIdentity(role)
      },
    } as unknown as Store
    const runtime = new Runtime(
      store,
      index === 0 ? 'Mac' : 'Windows',
      index === 0 ? diagnostics : undefined,
    )
    running.push(runtime)
    await runtime.init()
    return runtime
  }
  const host = await make('host', 0),
    client = await make('client', 1)
  return { host, client, ids, code, engine: mocks.engines[0]! }
}
async function pair(reverse = false, saveIdentity = async (_role: 'host' | 'client') => {}) {
  const result = await createPair(reverse, saveIdentity)
  const { host, client, code } = result
  await host.start()
  await client.start(code)
  await waitFor(
    () =>
      host.state.connected &&
      client.state.connected &&
      host.state.engine === 'ready' &&
      client.state.engine === 'ready',
  )
  return result
}
describe('two actual control-channel runtimes with simulated hardware', () => {
  it('refreshes live permissions without saving, restarting sharing or issuing input commands', async () => {
    const { host } = await pair()
    const save = vi.spyOn(host.store, 'save')
    let finish!: (value: boolean) => void
    const check = vi.spyOn(host.display, 'checkAccessibility').mockImplementationOnce(
      () =>
        new Promise<boolean>((resolve) => {
          finish = resolve
        }),
    )
    const starts = mocks.starts.length
    const first = host.refreshAccessibility()
    expect(host.refreshAccessibility()).toBe(first)
    expect(check).toHaveBeenCalledOnce()
    finish(false)
    await first
    expect(host.state.dependencies.accessibility).toBe(false)
    check.mockResolvedValueOnce(true)
    await host.refreshAccessibility()
    expect(host.state.dependencies.accessibility).toBe(true)
    expect(host.state.running).toBe(true)
    expect(host.state.connected).toBe(true)
    expect(host.state.engine).toBe('ready')
    expect(save).not.toHaveBeenCalled()
    expect(mocks.starts).toHaveLength(starts)
    expect(mocks.writes).toHaveLength(0)
    expect(mocks.injections).toHaveLength(0)
  })
  it('rejects unsupported platforms before engine or network startup', async () => {
    const { host } = await createPair()
    host.state.platform = 'linux'
    await expect(host.start()).rejects.toThrow('暂不支持此操作系统')
    expect(host.state.running).toBe(false)
    expect(mocks.starts).toHaveLength(0)
  })
  it('batches renderer snapshots while preserving immediate internal control notifications', async () => {
    vi.useFakeTimers()
    const { host } = await createPair()
    try {
      await vi.advanceTimersByTimeAsync(250)
      const published = vi.fn(),
        internal = vi.fn(),
        snapshot = vi.spyOn(host, 'snapshot')
      host.on('state', published)
      host.on('control-state', internal)
      for (let i = 0; i < 100; i++) host.log('info', `activity ${i}`)
      expect(internal).toHaveBeenCalledTimes(100)
      expect(published).not.toHaveBeenCalled()
      expect(snapshot).not.toHaveBeenCalled()
      await vi.advanceTimersByTimeAsync(250)
      expect(published).toHaveBeenCalledOnce()
      expect(snapshot).toHaveBeenCalledOnce()
      expect(published.mock.calls[0]![0].logs[0].message).toBe('activity 99')
    } finally {
      vi.useRealTimers()
    }
  })
  it('keeps last video source after an ambiguous remote DDC failure and blocks ownership fallback', async () => {
    const { host, client, ids, code, engine } = await createPair()
    host.state.config.crossScreenVideo = 'keep'
    host.state.config.layoutSide = 'left'
    host.state.config.retainedDisplayId = 'internal'
    host.pointer = { validate() {}, hasRetainedDisplay: () => true, async land() {} }
    await host.start()
    await client.start(code)
    await waitFor(() => host.state.engine === 'ready' && client.state.engine === 'ready')
    const outbound = host.switchTo('peer')
    engine.emit('switch', { from: ids[0], to: ids[1] })
    await outbound
    engine.emit('switch', { from: ids[1], to: ids[0] })
    await waitFor(() => host.state.activeDeviceId === ids[0] && !host.state.switching)
    vi.spyOn(client.display, 'setInput').mockRejectedValueOnce(new Error('DDC response lost'))
    await expect(host.switchTo('local')).rejects.toThrow('DDC response lost')
    expect(host.state.lastDisplayDeviceId).toBe(ids[1])
    expect(host.state.displayDeviceId).toBeNull()
    expect(host.state.displayStatus).toBe('uncertain')
    await waitFor(() => client.state.displayStatus === 'uncertain')
    const injections = mocks.injections.length,
      writes = mocks.writes.length
    await expect(host.switchTo('peer')).rejects.toThrow('外接屏输入状态不确定')
    expect(mocks.injections).toHaveLength(injections)
    expect(mocks.writes).toHaveLength(writes)
  })
  it('exports pairing failures with exception stacks, session context and no pairing key', async () => {
    const root = mkdtempSync(join(tmpdir(), 'screenbridge-runtime-log-'))
    const diagnostics = new DiagnosticLog(join(root, 'logs'))
    let host: Runtime | undefined, client: Runtime | undefined
    try {
      const result = await createPair(
        false,
        async (role) => {
          if (role === 'host') throw new Error('diagnostic persistence failure')
        },
        diagnostics,
      )
      host = result.host
      client = result.client
      await host.start()
      await client.start(result.code)
      await waitFor(() =>
        host!.state.logs.some((log) => log.message.includes('diagnostic persistence failure')),
      )
      const file = join(root, 'report.json')
      diagnostics.exportTo(file, { snapshot: diagnosticSnapshot(host.snapshot()) })
      const text = readFileSync(file, 'utf8'),
        report = JSON.parse(text)
      expect(text).toContain('diagnostic persistence failure')
      expect(text).toContain('runtime.test.ts')
      expect(text).toContain('sharing.start')
      expect(text).toContain('"session": 1')
      expect(text).not.toContain(result.code)
      expect(text).not.toContain(result.code.slice(4))
      expect(report.context.snapshot.config.role).toBe('host')
    } finally {
      await client?.stop()
      await host?.stop()
      rmSync(root, { recursive: true, force: true })
    }
  })
  it.each(['host', 'client'] as const)(
    'waits for slow %s identity persistence before starting either engine',
    async (role) => {
      const { host, client, ids } = await pair(false, async (savingRole) => {
        if (savingRole === role) await new Promise((resolve) => setTimeout(resolve, 100))
      })
      expect(mocks.starts.map((peer) => peer.id).sort()).toEqual([...ids].sort())
      expect(host.state.peer?.id).toBe(ids[1])
      expect(client.state.peer?.id).toBe(ids[0])
      expect(
        [...host.state.logs, ...client.state.logs].some((log) =>
          log.message.includes('Cannot read properties'),
        ),
      ).toBe(false)
    },
  )
  it('does not let a stopped session publish or start an engine after reconnecting', async () => {
    let release!: () => void,
      entered = false,
      saves = 0
    const gate = new Promise<void>((resolve) => {
      release = resolve
    })
    const { host, client, code, ids } = await createPair(false, async (role) => {
      if (role === 'host' && ++saves === 1) {
        entered = true
        await gate
      }
    })
    try {
      await host.start()
      await client.start(code)
      await waitFor(() => entered)
      await client.stop()
      await host.stop()
      expect(host.state.connected).toBe(false)
      expect(host.state.peer).toBeNull()
      expect(mocks.starts).toHaveLength(0)
      await host.start()
      await client.start(code)
      await waitFor(() => host.state.engine === 'ready' && client.state.engine === 'ready')
      release()
      await new Promise((resolve) => setImmediate(resolve))
      expect(mocks.starts).toHaveLength(2)
      expect(host.state.connected).toBe(true)
      expect(host.state.peer?.id).toBe(ids[1])
      expect(
        host.state.logs.filter((log) => log.message.includes('建立加密控制连接')),
      ).toHaveLength(1)
    } finally {
      release()
    }
  })
  it('rejects a failed identity save without launching engines and allows a later retry', async () => {
    let fail = true
    const { host, client, code } = await createPair(false, async (role) => {
      if (role === 'host' && fail) throw new Error('测试：身份保存失败')
    })
    await host.start()
    await client.start(code)
    await waitFor(() => host.state.logs.some((log) => log.message.includes('身份保存失败')))
    await waitFor(() => !host.state.connected && !client.state.connected)
    expect(mocks.starts).toHaveLength(0)
    await client.stop()
    await host.stop()
    fail = false
    await host.start()
    await client.start(code)
    await waitFor(() => host.state.engine === 'ready' && client.state.engine === 'ready')
    expect(mocks.starts).toHaveLength(2)
  })
  it('pairs and routes both display transitions through the outgoing machine', async () => {
    const { host, client, ids, engine } = await pair()
    engine.emit('switch', { from: ids[0], to: ids[1] })
    await waitFor(() => client.state.activeDeviceId === ids[1] && !client.state.switching)
    engine.emit('switch', { from: ids[1], to: ids[0] })
    await waitFor(() => client.state.activeDeviceId === ids[0] && !client.state.switching)
    expect(mocks.writes).toEqual([
      { machine: 'Mac', input: 1 },
      { machine: 'Windows', input: 3 },
    ])
    expect(host.store.value.trustedPeer?.id).toBe(ids[1])
    await client.stop()
    await waitFor(() => !host.state.connected)
    expect(host.state.activeDeviceId).toBeNull()
  })
  it('reports missing keyboard observation without treating it as a permission denial', async () => {
    const { host, engine } = await pair()
    const before = engine.listenerCount('switch')
    vi.useFakeTimers()
    try {
      const result = host.switchTo('peer')
      const checked = expect(result).rejects.toThrow('未收到 Deskflow 键鼠切换记录')
      await vi.advanceTimersByTimeAsync(12000)
      await checked
      expect(host.state.dependencies.accessibility).toBe(true)
      expect(host.state.engine).toBe('ready')
      expect(engine.listenerCount('switch')).toBe(before)
    } finally {
      vi.useRealTimers()
    }
  })
  it('distinguishes an observed keyboard switch from a delayed display write', async () => {
    const { host, engine, ids } = await pair()
    let release!: () => void
    vi.spyOn(host.display, 'setInput').mockImplementationOnce(
      () =>
        new Promise<void>((resolve) => {
          release = resolve
        }),
    )
    vi.useFakeTimers()
    try {
      const result = host.switchTo('peer')
      const checked = expect(result).rejects.toThrow('已收到 Deskflow 键鼠切换')
      engine.emit('switch', { from: ids[0], to: ids[1] })
      await vi.advanceTimersByTimeAsync(12000)
      await checked
    } finally {
      release?.()
      vi.useRealTimers()
    }
  })
  it('keeps mapping independent of host OS and rejects mismatched peer mappings', async () => {
    const { host, client, ids, engine } = await pair(true)
    engine.emit('switch', { from: ids[0], to: ids[1] })
    await waitFor(() => client.state.activeDeviceId === ids[1] && !client.state.switching)
    expect(mocks.writes).toEqual([{ machine: 'Mac', input: 3 }])
    await client.stop()
    await host.stop()
    client.state.config = { ...client.state.config, localInput: 9 }
    client.store.value.config = client.state.config
    await host.start()
    await client.start(host.state.pairingCode)
    await waitFor(
      () =>
        host.state.logs.some((l) => l.message.includes('映射不对应')) ||
        client.state.logs.some((l) => l.message.includes('映射不对应')),
    )
    expect(host.state.connected).toBe(false)
    expect(client.state.connected).toBe(false)
  })
  it('keeps VGA active during keyboard round trips and routes the later DDC write through Windows', async () => {
    const { host, client, ids, code, engine } = await createPair()
    host.state.config.crossScreenVideo = 'keep'
    host.state.config.retainedDisplayId = 'internal'
    host.state.config.layoutSide = 'left'
    const land = vi.fn(async () => {})
    host.pointer = { validate: vi.fn(), hasRetainedDisplay: () => true, land }
    await host.start()
    await client.start(code)
    await waitFor(() => host.state.engine === 'ready' && client.state.engine === 'ready')
    expect(host.state.displayDeviceId).toBeNull()
    const outbound = host.switchTo('peer')
    engine.emit('switch', { from: ids[0], to: ids[1] })
    await outbound
    expect(mocks.writes).toEqual([{ machine: 'Mac', input: 1 }])
    expect(host.state.displayDeviceId).toBe(ids[1])
    await waitFor(
      () =>
        client.state.activeDeviceId === ids[1] &&
        client.state.displayDeviceId === ids[1] &&
        !client.state.switching,
    )
    engine.emit('switch', { from: ids[1], to: ids[0] })
    await waitFor(() => client.state.activeDeviceId === ids[0] && !client.state.switching)
    expect(land).toHaveBeenCalledTimes(1)
    expect(host.state.displayDeviceId).toBe(ids[1])
    expect(client.state.displayDeviceId).toBe(ids[1])
    expect(mocks.writes).toHaveLength(1)
    // A video switch must still work when the keyboard is already on the target.
    await host.switchTo('local')
    expect(mocks.writes).toEqual([
      { machine: 'Mac', input: 1 },
      { machine: 'Windows', input: 3 },
    ])
    expect(host.state.displayDeviceId).toBe(ids[0])
  })
  it('uses the host shortcut on the receiver and coordinates a video return over the control channel', async () => {
    const { host, client, ids, code, engine } = await createPair()
    host.state.config.shortcut = 'control+alt+F10'
    client.state.config.shortcut = 'control+alt+F11'
    host.state.config.crossScreenVideo = 'keep'
    host.state.config.layoutSide = 'left'
    expect(client.switchShortcut).toBe('')
    await host.start()
    await client.start(code)
    await waitFor(() => host.state.engine === 'ready' && client.state.engine === 'ready')
    expect(host.switchShortcut).toBe('control+alt+F10')
    expect(client.switchShortcut).toBe('control+alt+F10')
    expect(client.state.config.shortcut).toBe('control+alt+F11')
    const outbound = host.toggleDisplay('shortcut')
    engine.emit('switch', { from: ids[0], to: ids[1] })
    await outbound
    await waitFor(() => client.state.displayDeviceId === ids[1] && !client.state.switching)
    const inbound = client.toggleDisplay('shortcut')
    await waitFor(() => mocks.injections.includes('host'))
    engine.emit('switch', { from: ids[1], to: ids[0] })
    await inbound
    expect(mocks.writes).toEqual([
      { machine: 'Mac', input: 1 },
      { machine: 'Windows', input: 3 },
    ])
    await waitFor(() => client.state.displayDeviceId === ids[0] && !client.state.switching)
    await client.stop()
    expect(client.switchShortcut).toBe('')
  })
  it('uses a native edge crossing with its fraction instead of a remembered hotkey jump', async () => {
    const { host, client, ids, code, engine } = await createPair()
    host.state.config.crossScreenVideo = 'keep'
    host.state.config.retainedDisplayId = 'internal'
    host.state.config.layoutSide = 'left'
    const crossToPeer = vi.fn(async (_position: number) => {
      engine.emit('switch', { from: ids[0], to: ids[1] })
    })
    host.pointer = { validate() {}, hasRetainedDisplay: () => true, async land() {}, crossToPeer }
    await host.start()
    await client.start(code)
    await waitFor(() => host.state.engine === 'ready' && client.state.engine === 'ready')
    for (const position of [0.1, 0.5, 0.9]) {
      await host.switchKeyboardToPeer(position)
      expect(crossToPeer).toHaveBeenLastCalledWith(position)
      expect(host.state.activeDeviceId).toBe(ids[1])
      engine.emit('switch', { from: ids[1], to: ids[0] })
      await waitFor(() => host.state.activeDeviceId === ids[0] && !host.state.switching)
    }
    expect(mocks.injections).toEqual([])
    // Keyboard-only round trips keep the shared panel on the receiver.
    expect(mocks.writes).toEqual([{ machine: 'Mac', input: 1 }])
  })
  it('allows the built-in seam to transfer keyboard control without issuing a video command', async () => {
    const { host, client, ids, code, engine } = await createPair()
    host.state.config.crossScreenVideo = 'keep'
    host.state.config.retainedDisplayId = 'internal'
    host.state.config.layoutSide = 'right'
    host.pointer = { validate() {}, hasRetainedDisplay: () => true, async land() {} }
    await host.start()
    await client.start(code)
    await waitFor(() => host.state.engine === 'ready' && client.state.engine === 'ready')
    const outbound = host.switchTo('peer')
    engine.emit('switch', { from: ids[0], to: ids[1] })
    await outbound
    engine.emit('switch', { from: ids[1], to: ids[0] })
    await waitFor(() => host.state.activeDeviceId === ids[0] && !host.state.switching)
    const crossing = host.switchKeyboardToPeer()
    engine.emit('switch', { from: ids[0], to: ids[1] })
    await crossing
    expect(host.state.activeDeviceId).toBe(ids[1])
    expect(host.state.displayDeviceId).toBe(ids[1])
    expect(mocks.writes).toEqual([{ machine: 'Mac', input: 1 }])
  })
  it('makes a native Mac outbound edge visible once and preserves Windows video on subsequent round trips', async () => {
    const { host, client, ids, code, engine } = await createPair()
    host.state.config.crossScreenVideo = 'keep'
    host.state.config.retainedDisplayId = 'internal'
    host.state.config.layoutSide = 'top'
    const land = vi.fn(async () => {})
    host.pointer = { validate() {}, hasRetainedDisplay: () => true, land }
    await host.start()
    await client.start(code)
    await waitFor(() => host.state.engine === 'ready' && client.state.engine === 'ready')
    engine.emit('switch', { from: ids[0], to: ids[1] })
    await waitFor(() => host.state.activeDeviceId === ids[1] && !host.state.switching)
    expect(mocks.writes).toEqual([{ machine: 'Mac', input: 1 }])
    engine.emit('switch', { from: ids[1], to: ids[0] })
    await waitFor(() => client.state.activeDeviceId === ids[0] && !client.state.switching)
    expect(land).toHaveBeenCalledOnce()
    expect(client.state.displayDeviceId).toBe(ids[1])
    engine.emit('switch', { from: ids[0], to: ids[1] })
    await waitFor(() => client.state.activeDeviceId === ids[1] && !client.state.switching)
    expect(client.state.displayDeviceId).toBe(ids[1])
    expect(mocks.writes).toHaveLength(1)
  })
  it.each([
    ['win32', 'win32'],
    ['win32', 'darwin'],
    ['darwin', 'win32'],
    ['darwin', 'darwin'],
  ] as const)(
    'supports %s host and %s receiver sharing one screen with bidirectional video following',
    async (hostOS, clientOS) => {
      const { host, client, ids, code, engine } = await createPair()
      host.state.platform = hostOS
      client.state.platform = clientOS
      host.state.config.shortcut = 'control+alt+F10'
      host.state.config.layoutSide = 'bottom'
      // Keeping video is safe only when the destination has another visible panel.
      host.state.config.crossScreenVideo = 'keep'
      await host.start()
      await client.start(code)
      await waitFor(() => host.state.engine === 'ready' && client.state.engine === 'ready')
      expect(host.state.peer?.platform).toBe(clientOS)
      expect(client.state.sessionLayout).toEqual({ hostSide: 'bottom', video: 'keep' })
      expect(client.state.config.layoutSide).toBe('off')
      expect(client.pointerMode).toBe(true)
      engine.emit('switch', { from: ids[0], to: ids[1] })
      await waitFor(() => client.state.activeDeviceId === ids[1] && !client.state.switching)
      expect(client.state.displayDeviceId).toBe(ids[1])
      engine.emit('switch', { from: ids[1], to: ids[0] })
      await waitFor(() => client.state.activeDeviceId === ids[0] && !client.state.switching)
      expect(mocks.writes).toEqual([
        { machine: 'Mac', input: 1 },
        { machine: 'Windows', input: 3 },
      ])
      expect(client.state.displayDeviceId).toBe(ids[0])
      await client.stop()
      await waitFor(() => !host.state.connected)
      expect(host.state.sessionLayout).toBeNull()
      expect(client.state.sessionLayout).toBeNull()
    },
  )
  it('lands on a Windows receiver independent screen and handles its outbound seam request', async () => {
    const { host, client, ids, code, engine } = await createPair()
    host.state.platform = 'win32'
    client.state.platform = 'win32'
    host.state.config.shortcut = 'control+alt+F10'
    host.state.config.layoutSide = 'right'
    host.state.config.crossScreenVideo = 'keep'
    client.state.config.retainedDisplayId = '42'
    const land = vi.fn(async () => {})
    client.pointer = { validate() {}, hasRetainedDisplay: () => true, land }
    await host.start()
    await client.start(code)
    await waitFor(() => host.state.engine === 'ready' && client.state.engine === 'ready')
    // Establish which computer the shared monitor is displaying.
    engine.emit('switch', { from: ids[0], to: ids[1] })
    await waitFor(() => host.state.activeDeviceId === ids[1] && !host.state.switching)
    engine.emit('switch', { from: ids[1], to: ids[0] })
    await waitFor(() => client.state.activeDeviceId === ids[0] && !client.state.switching)
    expect(host.state.displayDeviceId).toBe(ids[0])
    const writes = mocks.writes.length
    engine.emit('switch', { from: ids[0], to: ids[1] })
    await waitFor(() => client.state.activeDeviceId === ids[1] && !client.state.switching)
    expect(land).toHaveBeenCalledOnce()
    expect(mocks.writes).toHaveLength(writes)
    expect(client.state.displayDeviceId).toBe(ids[0])
    const crossing = client.switchKeyboardToPeer()
    await waitFor(() => mocks.injections.includes('host'))
    engine.emit('switch', { from: ids[1], to: ids[0] })
    await crossing
    expect(host.state.activeDeviceId).toBe(ids[0])
    expect(mocks.writes).toHaveLength(writes)
  })
  it('follows video even when independent screens exist if the user chooses follow', async () => {
    const { host, client, ids, code, engine } = await createPair()
    host.state.config.layoutSide = 'left'
    host.state.config.crossScreenVideo = 'follow'
    const land = vi.fn(async () => {})
    host.pointer = { validate() {}, hasRetainedDisplay: () => true, land }
    await host.start()
    await client.start(code)
    await waitFor(() => host.state.engine === 'ready' && client.state.engine === 'ready')
    engine.emit('switch', { from: ids[0], to: ids[1] })
    await waitFor(() => host.state.activeDeviceId === ids[1] && !host.state.switching)
    engine.emit('switch', { from: ids[1], to: ids[0] })
    await waitFor(() => client.state.activeDeviceId === ids[0] && !client.state.switching)
    expect(mocks.writes).toHaveLength(2)
    expect(land).not.toHaveBeenCalled()
  })
  it('rejects an old Windows host F12 before starting a session and accepts a user-selected F10', async () => {
    const { host } = await createPair()
    host.state.platform = 'win32'
    await expect(host.start()).rejects.toThrow('F12 是 Windows 保留键')
    expect(host.state.running).toBe(false)
    await host.save({ ...host.state.config, shortcut: 'control+alt+F10' })
    await host.start()
    expect(host.state.listening).toBe(true)
  })
  it('saves a Windows receiver-to-host role change before requiring a supported shortcut at start', async () => {
    const { client } = await createPair()
    client.state.platform = 'win32'
    expect(client.state.config.role).toBe('client')
    expect(client.state.config.shortcut).toBe('control+alt+F12')
    await client.save({ ...client.state.config, role: 'host' })
    expect(client.state.config.role).toBe('host')
    expect(client.store.value.config.role).toBe('host')
    expect(client.state.config.shortcut).toBe('control+alt+F12')
    await expect(client.start()).rejects.toThrow('F12 是 Windows 保留键')
    expect(client.state.running).toBe(false)
    await client.save({ ...client.state.config, shortcut: 'control+alt+F10' })
    expect(client.store.value.config.shortcut).toBe('control+alt+F10')
    await client.start()
    expect(client.state.listening).toBe(true)
  })
  it('cancels a pending keyboard command on stop without leaving listeners or issuing DDC', async () => {
    const { host, engine } = await pair()
    const before = engine.listenerCount('switch')
    const command = host.switchTo('peer')
    const rejected = expect(command).rejects.toThrow('共享会话已结束')
    await host.stop()
    await rejected
    expect(engine.listenerCount('switch')).toBe(before)
    expect(mocks.writes).toHaveLength(0)
    expect(host.state.displayDeviceId).toBeNull()
  })
})
