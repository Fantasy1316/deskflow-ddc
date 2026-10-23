import { afterEach, describe, expect, it, vi } from 'vitest'
import { EventEmitter } from 'node:events'
import { DeskflowEngine } from '../electron/core/engine'
import { defaults } from '../electron/core/config'
import type { Peer } from '../src/shared/types'

const mocks = vi.hoisted(() => ({ spawn: vi.fn(), connect: vi.fn(), cleanup: vi.fn() }))
vi.mock('node:child_process', () => ({ spawn: mocks.spawn }))
vi.mock('node:net', () => ({ connect: mocks.connect }))
vi.mock('../electron/core/engine-launch', () => ({
  engineLaunch: async () => ({
    env: { TMPDIR: '/private/test/owned/' },
    directory: '/private/test/owned',
    ipcPath: '/private/test/owned/deskflow-core',
    cleanup: mocks.cleanup,
  }),
}))
vi.mock('../electron/core/engine-preflight', () => ({
  assertNoRunningEngine: async () => {},
  assertPortAvailable: async () => {},
}))
afterEach(() => {
  vi.useRealTimers()
  vi.restoreAllMocks()
  vi.clearAllMocks()
})

async function start() {
  vi.useFakeTimers()
  const child = Object.assign(new EventEmitter(), {
    pid: 123,
    stdout: new EventEmitter(),
    stderr: new EventEmitter(),
    kill: vi.fn(),
  })
  const socket = Object.assign(new EventEmitter(), {
    setEncoding: vi.fn(),
    write: vi.fn(),
    destroy: vi.fn(),
  })
  mocks.spawn.mockReturnValue(child)
  mocks.connect.mockReturnValue(socket)
  mocks.cleanup.mockResolvedValue(undefined)
  const engine = new DeskflowEngine('/test')
  vi.spyOn(engine, 'prepare').mockResolvedValue({
    binary: '/deskflow-core',
    settings: '/test/Deskflow.conf',
  })
  const states: string[] = [],
    messages: string[] = [],
    traces: string[] = []
  engine.on('state', (value) => states.push(value))
  engine.on('diagnostic', (value) => messages.push(value))
  engine.on('trace', (value) => traces.push(value))
  await engine.start(
    { ...defaults('test'), role: 'host' },
    'a',
    { id: 'b', fingerprint: 'a'.repeat(64) } as Peer,
    '',
  )
  return { engine, child, socket, states, messages, traces }
}
describe('engine startup stall handling', () => {
  it('cleans IPC and startup timers when spawning fails before an exit event', async () => {
    const { engine, child, socket, states, messages } = await start()
    child.emit('error', new Error('spawn ENOENT'))
    child.emit('close', -2)
    await Promise.resolve()
    await Promise.resolve()
    expect(states.at(-1)).toBe('error')
    expect(messages.join(' ')).toContain('ENOENT')
    expect(socket.destroy).toHaveBeenCalled()
    expect(mocks.cleanup).toHaveBeenCalledOnce()
    expect(vi.getTimerCount()).toBe(0)
    await expect(engine.assertAvailable(defaults('test'))).resolves.toBeUndefined()
  })
  it('retains an unresponsive child after SIGKILL timeout and blocks a restart until exit', async () => {
    const { engine, child } = await start()
    const stopped = engine.stop(),
      checked = expect(stopped).rejects.toThrow('仍未退出')
    await vi.advanceTimersByTimeAsync(3000)
    await checked
    expect(child.kill.mock.calls).toEqual([['SIGTERM'], ['SIGKILL']])
    await expect(engine.assertAvailable(defaults('test'))).rejects.toThrow('已经运行')
    child.emit('exit', null, 'SIGKILL')
    child.emit('close')
    await expect(engine.assertAvailable(defaults('test'))).resolves.toBeUndefined()
  })
  it('parses split IPC hello, negotiates once, and rejects the wrong peer fingerprint', async () => {
    const { child, socket, messages, states } = await start()
    socket.emit('connect')
    socket.emit('data', 'versionMis')
    socket.emit('data', 'match=1.27.0\nhello=1.27.0\n')
    expect(socket.write.mock.calls).toEqual([['hello=screenbridge-probe\n'], ['hello=1.27.0\n']])
    socket.emit('data', `peerFingerprint=${'b'.repeat(64)}\nconnectionState=Connected\n`)
    expect(messages.at(-1)).toContain('未通过配对身份检查')
    expect(states).not.toContain('ready')
    child.emit('exit', 1)
    child.emit('close')
    await Promise.resolve()
  })
  it('connects IPC in the child temp namespace and ends a silent startup stall', async () => {
    const { child, states, messages, traces } = await start()
    expect(mocks.spawn.mock.calls[0]![2].env.TMPDIR).toBe('/private/test/owned/')
    expect(mocks.connect).toHaveBeenCalledWith('/private/test/owned/deskflow-core')
    await vi.advanceTimersByTimeAsync(18000)
    expect(states.at(-1)).toBe('error')
    expect(child.kill).toHaveBeenCalledOnce()
    expect(messages.at(-1)).toContain('尚未建立本机 IPC')
    expect(traces).toContain('engine.startupTimeout')
    expect(mocks.cleanup).not.toHaveBeenCalled()
    child.emit('close')
    await Promise.resolve()
    expect(mocks.cleanup).toHaveBeenCalledOnce()
  })
  it('keeps a listening host alive and reports a different stage from startup failure', async () => {
    const { child, socket, states, messages } = await start()
    socket.emit('data', 'hello=1.27.0\nconnectionState=Listening\n')
    await vi.advanceTimersByTimeAsync(18000)
    expect(child.kill).not.toHaveBeenCalled()
    expect(states.at(-1)).toBe('starting')
    expect(messages.at(-1)).toContain('本机引擎已启动')
  })
  it('cancels the timeout after a confirmed connection', async () => {
    const { child, socket, states, messages } = await start()
    socket.emit('data', 'hello=1.27.0\nconnectionState=Connected\n')
    await vi.advanceTimersByTimeAsync(20000)
    expect(states.at(-1)).toBe('ready')
    expect(messages).toEqual([])
    expect(child.kill).not.toHaveBeenCalled()
  })
})
