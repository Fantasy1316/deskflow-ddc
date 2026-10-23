import { afterEach, describe, expect, it, vi } from 'vitest'
import { EventEmitter } from 'node:events'
import { WindowsHelper } from '../electron/platform/windows-helper'

const mocks = vi.hoisted(() => ({ spawn: vi.fn() }))
vi.mock('node:child_process', () => ({ spawn: mocks.spawn }))
afterEach(() => {
  vi.useRealTimers()
  vi.clearAllMocks()
})
function setup(respond = true) {
  const children: ReturnType<typeof makeChild>[] = []
  function makeChild() {
    const stdout = Object.assign(new EventEmitter(), { setEncoding: vi.fn() }),
      stderr = new EventEmitter()
    const stdin = Object.assign(new EventEmitter(), {
      write: vi.fn((text: string, callback: (error?: Error) => void) => {
        const command = JSON.parse(text)
        callback()
        if (respond)
          queueMicrotask(() =>
            stdout.emit(
              'data',
              JSON.stringify({ id: command.id, ok: true, result: { action: command.action } }) +
                '\n',
            ),
          )
      }),
    })
    const child = Object.assign(new EventEmitter(), {
      stdin,
      stdout,
      stderr,
      exitCode: null as number | null,
      signalCode: null as string | null,
      kill: vi.fn((signal: string) => {
        child.signalCode = signal
        child.emit('exit', null, signal)
        child.emit('close')
        return true
      }),
    })
    return child
  }
  mocks.spawn.mockImplementation(() => {
    const child = makeChild()
    children.push(child)
    return child
  })
  return { helper: new WindowsHelper('C:/private/monitor.ps1'), children }
}
describe('persistent Windows native helper lifecycle', () => {
  it('blocks subsequent commands if a helper does not exit after forced termination', async () => {
    vi.useFakeTimers()
    const { helper, children } = setup()
    await helper.request('list')
    const child = children[0]!
    child.kill.mockImplementation(() => true)
    const closing = helper.close(),
      checked = expect(closing).rejects.toThrow('仍未退出')
    await vi.advanceTimersByTimeAsync(3000)
    await checked
    await expect(helper.request('set', { value: 3 })).rejects.toThrow('暂停后续命令')
    expect(child.stdin.write).toHaveBeenCalledOnce()
    expect(mocks.spawn).toHaveBeenCalledOnce()
    child.signalCode = 'SIGKILL'
    child.emit('exit', null, 'SIGKILL')
    child.emit('close')
    await helper.close()
    expect(vi.getTimerCount()).toBe(0)
  })
  it('invalidates a new request waiting for an earlier shutdown when stopped again', async () => {
    const { helper, children } = setup()
    await helper.request('list')
    const child = children[0]!
    child.kill.mockImplementation(() => true)
    const closing = helper.close()
    const waiting = helper.request('pointer'),
      checked = expect(waiting).rejects.toThrow('会话已结束')
    await Promise.resolve()
    await Promise.resolve()
    const again = helper.close()
    child.signalCode = 'SIGTERM'
    child.emit('exit', null, 'SIGTERM')
    child.emit('close')
    await closing
    await again
    await checked
    expect(mocks.spawn).toHaveBeenCalledOnce()
  })
  it('reuses one private process and serializes native operations', async () => {
    const { helper, children } = setup()
    const [a, b] = await Promise.all([
      helper.request('set', { monitorId: 'display', value: 3 }),
      helper.request('pointer', { x: -10, y: 40 }),
    ])
    expect(JSON.parse(a).action).toBe('set')
    expect(JSON.parse(b).action).toBe('pointer')
    expect(mocks.spawn).toHaveBeenCalledOnce()
    expect(children[0]!.stdin.write).toHaveBeenCalledTimes(2)
    expect(mocks.spawn.mock.calls[0]![1]).toContain('serve')
    await helper.close()
    expect(children[0]!.kill).toHaveBeenCalledWith('SIGTERM')
  })
  it('cancels active and queued operations on close without resurrecting the helper', async () => {
    const { helper, children } = setup(false)
    const active = helper.request('set', { value: 1 }),
      queued = helper.request('pointer', { x: 0, y: 0 })
    const first = expect(active).rejects.toThrow('助手已停止'),
      second = expect(queued).rejects.toThrow('会话已结束')
    await Promise.resolve()
    await helper.close()
    await first
    await second
    expect(mocks.spawn).toHaveBeenCalledOnce()
    expect(children[0]!.stdin.write).toHaveBeenCalledOnce()
  })
  it('terminates a timed-out helper and never automatically retries a possibly executed DDC command', async () => {
    vi.useFakeTimers()
    const { helper, children } = setup(false)
    const request = helper.request('set', { value: 3 }),
      checked = expect(request).rejects.toThrow('响应超时')
    await vi.advanceTimersByTimeAsync(10000)
    await checked
    expect(mocks.spawn).toHaveBeenCalledOnce()
    expect(children[0]!.stdin.write).toHaveBeenCalledOnce()
    await helper.close()
    expect(vi.getTimerCount()).toBe(0)
  })
  it('handles split response frames and rejects malformed JSON', async () => {
    const { helper, children } = setup(false)
    const request = helper.request('list')
    await Promise.resolve()
    children[0]!.stdout.emit('data', '{"id":1,"ok":true,"res')
    children[0]!.stdout.emit('data', 'ult":[]}\n')
    expect(await request).toBe('[]')
    const invalid = helper.request('pointer'),
      checked = expect(invalid).rejects.toThrow()
    await Promise.resolve()
    await Promise.resolve()
    children[0]!.stdout.emit('data', 'invalid\n')
    await checked
    await helper.close()
  })
})
