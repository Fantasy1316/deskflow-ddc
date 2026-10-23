import { afterEach, describe, expect, it, vi } from 'vitest'
import { spawn, type ChildProcess } from 'node:child_process'
import { EventEmitter, once } from 'node:events'
import { stopOwnedProcess } from '../electron/core/process-stop'
afterEach(() => vi.useRealTimers())
describe('owned process termination', () => {
  it.skipIf(process.platform === 'win32')(
    'force-kills a real child that ignores SIGTERM and awaits its exit',
    async () => {
      const child = spawn(
        process.execPath,
        [
          '-e',
          "process.on('SIGTERM',()=>{});process.stdout.write('ready');setInterval(()=>{},1000)",
        ],
        { stdio: ['ignore', 'pipe', 'ignore'] },
      )
      try {
        await once(child.stdout!, 'data')
        await stopOwnedProcess(child, 50, 2000)
        expect(child.signalCode).toBe('SIGKILL')
      } finally {
        if (child.exitCode == null && child.signalCode == null) child.kill('SIGKILL')
      }
    },
  )
  it('rejects when exit never arrives, removes waiters and allows a later attempt', async () => {
    vi.useFakeTimers()
    const child = Object.assign(new EventEmitter(), { kill: vi.fn() })
    const result = stopOwnedProcess(child as unknown as ChildProcess)
    const checked = expect(result).rejects.toThrow('仍未退出')
    await vi.advanceTimersByTimeAsync(3000)
    await checked
    expect(child.kill.mock.calls).toEqual([['SIGTERM'], ['SIGKILL']])
    expect(child.listenerCount('exit')).toBe(0)
    const second = stopOwnedProcess(child as unknown as ChildProcess)
    child.emit('exit')
    await second
    expect(vi.getTimerCount()).toBe(0)
  })
})
