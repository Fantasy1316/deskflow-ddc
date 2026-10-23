import { afterEach, describe, expect, it, vi } from 'vitest'
import { EventEmitter } from 'node:events'
import type { Runtime } from '../electron/core/runtime'
import { PointerBridge } from '../electron/platform/pointer'

const desktop = vi.hoisted(() => ({
  cursor: { x: 1400, y: 500 },
  scale: 1,
  displays: [
    {
      id: 42,
      label: 'Independent panel',
      internal: true,
      bounds: { x: 0, y: 0, width: 1512, height: 982 },
    },
  ],
  events: new Map<string, () => void>(),
  register: vi.fn(() => true),
  unregister: vi.fn(),
}))
vi.mock('electron', () => ({
  screen: {
    getAllDisplays: () => desktop.displays,
    getCursorScreenPoint: () => desktop.cursor,
    dipToScreenPoint: (point: { x: number; y: number }) => ({
      x: point.x * desktop.scale,
      y: point.y * desktop.scale,
    }),
    on: (event: string, callback: () => void) => desktop.events.set(event, callback),
  },
  globalShortcut: { register: desktop.register, unregister: desktop.unregister },
}))
function setup(role = 'host', platform = 'darwin') {
  desktop.scale = 1
  desktop.displays = [
    {
      id: 42,
      label: 'Independent panel',
      internal: true,
      bounds: { x: 0, y: 0, width: 1512, height: 982 },
    },
  ]
  const runtime = Object.assign(new EventEmitter(), {
    pointerMode: true,
    layout: { hostSide: 'left', video: 'keep' },
    state: {
      platform,
      cursorDisplays: [],
      connected: true,
      running: true,
      engine: 'ready',
      switching: false,
      activeDeviceId: 'local',
      deviceId: 'local',
      displayDeviceId: 'peer',
      peer: { id: 'peer' },
      config: { role, layoutSide: 'left', retainedDisplayId: '42', shortcut: 'control+alt+F10' },
    },
    display: { landOnScreen: vi.fn(async () => {}) },
    snapshot: vi.fn(() => ({})),
    stop: vi.fn(async () => {}),
    switchKeyboardToPeer: vi.fn(async () => {}),
    toggleDisplay: vi.fn(async () => {}),
    log: vi.fn(),
  })
  const bridge = new PointerBridge(runtime as unknown as Runtime)
  runtime.emit('state')
  return { runtime, bridge }
}
function disconnect(runtime: ReturnType<typeof setup>['runtime']) {
  runtime.state.connected = false
  runtime.emit('state')
}
afterEach(() => {
  vi.useRealTimers()
  vi.clearAllMocks()
  desktop.events.clear()
})
describe('platform-neutral independent display cursor bridge', () => {
  it('backs off after hotkey failure without recursive registration, then recovers', async () => {
    vi.useFakeTimers()
    desktop.register.mockReturnValueOnce(false).mockReturnValueOnce(false).mockReturnValue(true)
    const { runtime } = setup()
    runtime.log.mockImplementation(() => runtime.emit('state'))
    for (let i = 0; i < 10; i++) runtime.emit('state')
    expect(desktop.register).toHaveBeenCalledTimes(1)
    await vi.advanceTimersByTimeAsync(1000)
    expect(desktop.register).toHaveBeenCalledTimes(2)
    await vi.advanceTimersByTimeAsync(2000)
    expect(desktop.register).toHaveBeenCalledTimes(3)
    await vi.advanceTimersByTimeAsync(4000)
    expect(desktop.register).toHaveBeenCalledTimes(3)
    disconnect(runtime)
    expect(vi.getTimerCount()).toBe(0)
  })
  it('cancels registration retries on disconnect', async () => {
    vi.useFakeTimers()
    desktop.register.mockReturnValueOnce(false)
    const { runtime } = setup()
    disconnect(runtime)
    await vi.advanceTimersByTimeAsync(30000)
    expect(desktop.register).toHaveBeenCalledTimes(1)
    expect(vi.getTimerCount()).toBe(0)
  })
  it.each(['darwin', 'win32'])(
    'crosses the selected panel seam on a %s host while the aggregate desktop extends farther',
    async (platform) => {
      vi.useFakeTimers()
      const { runtime } = setup('host', platform)
      desktop.cursor = { x: 1490, y: 500 }
      await vi.advanceTimersByTimeAsync(16)
      desktop.cursor = { x: 1600, y: 500 }
      await vi.advanceTimersByTimeAsync(16)
      expect(runtime.switchKeyboardToPeer).toHaveBeenCalledTimes(1)
      expect(runtime.display.landOnScreen).not.toHaveBeenCalled()
      runtime.state.activeDeviceId = 'peer'
      await vi.advanceTimersByTimeAsync(700)
      expect(runtime.switchKeyboardToPeer).toHaveBeenCalledTimes(1)
      disconnect(runtime)
      expect(desktop.unregister).toHaveBeenCalledWith('Control+Alt+F10')
      expect(vi.getTimerCount()).toBe(0)
    },
  )
  it('does not warp after disconnect/reconnect invalidates an in-flight landing', async () => {
    vi.useFakeTimers()
    const { runtime, bridge } = setup()
    const pending = bridge.land()
    disconnect(runtime)
    runtime.state.connected = true
    runtime.emit('state')
    await vi.advanceTimersByTimeAsync(80)
    await pending
    expect(runtime.display.landOnScreen).not.toHaveBeenCalled()
    const current = bridge.land()
    await vi.advanceTimersByTimeAsync(80)
    await current
    expect(runtime.display.landOnScreen).toHaveBeenCalledWith({ x: 1488, y: 500 })
    disconnect(runtime)
  })
  it('does not transfer at the independent seam while the shared monitor shows this computer', async () => {
    vi.useFakeTimers()
    const { runtime } = setup()
    runtime.state.displayDeviceId = 'local'
    desktop.cursor = { x: 1490, y: 500 }
    await vi.advanceTimersByTimeAsync(16)
    desktop.cursor = { x: 1600, y: 500 }
    await vi.advanceTimersByTimeAsync(16)
    expect(runtime.switchKeyboardToPeer).not.toHaveBeenCalled()
    expect(runtime.display.landOnScreen).not.toHaveBeenCalled()
    const handler = desktop.register.mock.calls[0]?.[1] as (() => void) | undefined
    handler?.()
    expect(runtime.toggleDisplay).toHaveBeenCalledTimes(1)
    disconnect(runtime)
  })
  it('reverses the host layout on the receiver and leaves its public hotkey to the host', async () => {
    vi.useFakeTimers()
    const { runtime } = setup('client', 'win32')
    desktop.cursor = { x: 40, y: 300 }
    await vi.advanceTimersByTimeAsync(16)
    desktop.cursor = { x: 0, y: 300 }
    await vi.advanceTimersByTimeAsync(16)
    expect(runtime.switchKeyboardToPeer).toHaveBeenCalledOnce()
    expect(desktop.register).not.toHaveBeenCalled()
    disconnect(runtime)
  })
  it('lands on an external Windows panel with negative coordinates and physical DPI conversion', async () => {
    vi.useFakeTimers()
    const { runtime, bridge } = setup('client', 'win32')
    desktop.displays = [
      {
        id: 42,
        label: 'Second monitor',
        internal: false,
        bounds: { x: -1200, y: -800, width: 1200, height: 800 },
      },
    ]
    desktop.scale = 2
    desktop.cursor = { x: 100, y: 200 }
    bridge.validate()
    expect(bridge.hasRetainedDisplay()).toBe(true)
    const pending = bridge.land()
    await vi.advanceTimersByTimeAsync(80)
    await pending
    // Receiver is to the right of host; enter its left edge.
    expect(runtime.display.landOnScreen).toHaveBeenCalledWith({ x: -2352, y: -48 })
    disconnect(runtime)
  })
  it('does not sample without an independent screen and rejects a disconnected selection', () => {
    vi.useFakeTimers()
    const { runtime, bridge } = setup()
    runtime.state.config.retainedDisplayId = ''
    runtime.emit('state')
    expect(vi.getTimerCount()).toBe(0)
    expect(bridge.hasRetainedDisplay()).toBe(false)
    runtime.state.config.retainedDisplayId = 'missing'
    expect(() => bridge.validate()).toThrow('未发现所选独立屏幕')
    disconnect(runtime)
  })
  it('stops sharing when the selected independent screen disappears', () => {
    vi.useFakeTimers()
    const { runtime } = setup()
    desktop.displays = []
    desktop.events.get('display-removed')?.()
    expect(runtime.stop).toHaveBeenCalledOnce()
    expect(runtime.state.cursorDisplays).toEqual([])
    disconnect(runtime)
  })
})
