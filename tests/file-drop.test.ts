import { afterEach, expect, it, vi } from 'vitest'
const bridge = vi.hoisted(() => ({
  listen: vi.fn(),
  mounted: [] as (() => Promise<void>)[],
  unmounted: [] as (() => void)[],
}))
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true }))
vi.mock('@tauri-apps/api/event', () => ({ listen: bridge.listen }))
vi.mock('vue', async (original) => ({
  ...(await original<typeof import('vue')>()),
  onMounted: (f: () => Promise<void>) => bridge.mounted.push(f),
  onUnmounted: (f: () => void) => bridge.unmounted.push(f),
}))
import { useFileDrop } from '../src/composables/useFileDrop'
type Drag = {
  kind: 'enter' | 'over' | 'drop' | 'leave'
  position?: { x: number; y: number }
  token?: string
  error?: string
}
function fixture() {
  const stop = vi.fn()
  let handle!: (event: { payload: Drag }) => void
  bridge.listen.mockImplementation((_event, callback) => {
    handle = callback
    return Promise.resolve(stop)
  })
  let disabled = false
  const send = vi.fn()
  const element = {
    getBoundingClientRect: () => ({ left: 10, top: 20, right: 210, bottom: 220 }),
  } as HTMLElement
  const state = useFileDrop(
    () => element,
    () => disabled,
    send,
  )
  return {
    state,
    send,
    stop,
    event: (payload: Drag) => handle({ payload }),
    disable: () => {
      disabled = true
    },
  }
}
afterEach(() => {
  bridge.unmounted.forEach((f) => f())
  bridge.mounted.length = bridge.unmounted.length = 0
  vi.resetAllMocks()
  vi.unstubAllGlobals()
})
it('uses normalized logical coordinates without scaling them again on Retina', async () => {
  vi.stubGlobal('devicePixelRatio', 2)
  const f = fixture()
  await bridge.mounted[0]!()
  f.event({ kind: 'enter', position: { x: 40, y: 60 } })
  expect(f.state.dragging.value).toBe(true)
  f.event({ kind: 'over', position: { x: 220, y: 60 } })
  expect(f.state.dragging.value).toBe(false)
  f.event({ kind: 'drop', position: { x: 500, y: 60 }, token: 'outside' })
  f.event({ kind: 'drop', token: 'missing-position' })
  expect(f.send).not.toHaveBeenCalled()
  f.event({ kind: 'over', position: { x: 200, y: 200 } })
  expect(f.state.dragging.value).toBe(true)
  f.event({ kind: 'drop', position: { x: 40, y: 60 }, token: 'native-token' })
  expect(f.send).toHaveBeenCalledExactlyOnceWith('native-token')
  expect(f.state.dragging.value).toBe(false)
})
it('ignores disabled drops and clears highlighting when the drag leaves', async () => {
  const f = fixture()
  await bridge.mounted[0]!()
  f.event({ kind: 'over', position: { x: 20, y: 30 } })
  expect(f.state.dragging.value).toBe(true)
  f.event({ kind: 'leave' })
  expect(f.state.dragging.value).toBe(false)
  f.disable()
  f.event({ kind: 'enter', position: { x: 20, y: 30 } })
  f.event({ kind: 'drop', position: { x: 20, y: 30 }, token: 'disabled' })
  expect(f.state.dragging.value).toBe(false)
  expect(f.send).not.toHaveBeenCalled()
})
it('shows a native validation error without sending files and unsubscribes on disposal', async () => {
  const f = fixture()
  await bridge.mounted[0]!()
  f.event({ kind: 'drop', position: { x: 20, y: 30 }, error: '最多拖入 256 个文件或目录' })
  expect(f.state.error.value).toContain('256')
  expect(f.send).not.toHaveBeenCalled()
  bridge.unmounted[0]!()
  expect(f.stop).toHaveBeenCalledOnce()
  f.event({ kind: 'drop', position: { x: 20, y: 30 }, token: 'late' })
  expect(f.send).not.toHaveBeenCalled()
})
it('removes a listener that finishes registration after the page was left', async () => {
  let resolve!: (stop: () => void) => void
  const f = fixture()
  bridge.listen.mockReturnValue(
    new Promise<() => void>((r) => {
      resolve = r
    }),
  )
  const pending = bridge.mounted[0]!()
  bridge.unmounted[0]!()
  resolve(f.stop)
  await pending
  expect(f.stop).toHaveBeenCalledOnce()
})
