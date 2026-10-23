import { afterEach, describe, expect, it, vi } from 'vitest'
import { DiagnosticLines } from '../electron/core/diagnostic-lines'
afterEach(() => vi.useRealTimers())
describe('engine diagnostic flood control', () => {
  it('coalesces identical messages with different Deskflow timestamps', () => {
    vi.useFakeTimers()
    const emit = vi.fn(),
      lines = new DiagnosticLines(emit)
    lines.report('[2026-10-22T15:04:01.123] ERROR cannot connect')
    lines.report('[2026-10-22T15:04:01.456] ERROR cannot connect')
    lines.flush()
    expect(emit.mock.calls).toEqual([
      ['ERROR cannot connect'],
      ['ERROR cannot connect（本秒共 2 次）'],
    ])
  })
  it('keeps the first error durable immediately and summarizes repeated lines', async () => {
    vi.useFakeTimers()
    const emit = vi.fn(),
      lines = new DiagnosticLines(emit)
    for (let i = 0; i < 1000; i++) lines.report('ERROR cannot connect')
    expect(emit).toHaveBeenCalledExactlyOnceWith('ERROR cannot connect')
    await vi.advanceTimersByTimeAsync(1000)
    expect(emit).toHaveBeenCalledTimes(2)
    expect(emit.mock.calls[1]![0]).toContain('1000 次')
    expect(vi.getTimerCount()).toBe(0)
  })
  it('bounds distinct errors and flushes counts on engine shutdown', () => {
    vi.useFakeTimers()
    const emit = vi.fn(),
      lines = new DiagnosticLines(emit)
    for (let i = 0; i < 1000; i++) lines.report(`ERROR ${i}`)
    expect(emit).toHaveBeenCalledTimes(20)
    lines.flush()
    expect(emit.mock.calls.at(-1)![0]).toContain('980 条')
    expect(vi.getTimerCount()).toBe(0)
  })
})
