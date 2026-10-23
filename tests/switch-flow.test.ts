import { describe, expect, it, vi } from 'vitest'
import { SwitchFlow, type SwitchEffects } from '../electron/core/switch-flow'
function setup(overrides: Partial<SwitchEffects> = {}) {
  const effects: SwitchEffects = {
    writeInput: vi.fn(async () => {}),
    restoreKeyboard: vi.fn(async () => {}),
    changed: vi.fn(),
    finished: vi.fn(),
    ...overrides,
  }
  return { effects, flow: new SwitchFlow(new Set(['mac', 'windows']), effects) }
}
describe('coordinating hardware with keyboard focus', () => {
  it('asks the outgoing computer to change the monitor in both directions', async () => {
    const { flow, effects } = setup()
    await flow.observe({ from: 'mac', to: 'windows' })
    await flow.observe({ from: 'windows', to: 'mac' })
    expect(effects.writeInput).toHaveBeenNthCalledWith(1, 'mac', 'windows')
    expect(effects.writeInput).toHaveBeenNthCalledWith(2, 'windows', 'mac')
    expect(effects.restoreKeyboard).not.toHaveBeenCalled()
    expect(effects.changed).toHaveBeenLastCalledWith('mac', false)
  })
  it('restores keyboard focus after DDC failure without issuing another DDC write', async () => {
    const { flow, effects } = setup({
      writeInput: vi.fn(async () => {
        throw new Error('DDC offline')
      }),
    })
    await flow.observe({ from: 'mac', to: 'windows' })
    expect(effects.restoreKeyboard).toHaveBeenCalledWith('mac')
    expect(effects.finished).toHaveBeenCalledWith({ from: 'mac', to: 'windows' }, 'DDC offline')
    await flow.observe({ from: 'windows', to: 'mac' }) // observed compensation event
    expect(effects.writeInput).toHaveBeenCalledTimes(1)
  })
  it('serializes rapid switches instead of sending competing DDC writes', async () => {
    const order: string[] = []
    const { flow } = setup({
      writeInput: async (from, to) => {
        order.push('begin:' + from)
        await new Promise((r) => setTimeout(r, 5))
        order.push('end:' + to)
      },
    })
    await Promise.all([
      flow.observe({ from: 'mac', to: 'windows' }),
      flow.observe({ from: 'windows', to: 'mac' }),
    ])
    expect(order).toEqual(['begin:mac', 'end:windows', 'begin:windows', 'end:mac'])
  })
  it('ignores unknown devices and same-device events', async () => {
    const { flow, effects } = setup()
    await flow.observe({ from: 'mac', to: 'intruder' })
    await flow.observe({ from: 'mac', to: 'mac' })
    expect(effects.writeInput).not.toHaveBeenCalled()
  })
  it('reports unknown focus when keyboard recovery cannot be confirmed', async () => {
    const { flow, effects } = setup({
      writeInput: async () => {
        throw new Error('offline')
      },
      restoreKeyboard: async () => {
        throw new Error('no confirmation')
      },
    })
    await flow.observe({ from: 'mac', to: 'windows' })
    expect(effects.changed).toHaveBeenLastCalledWith(null, false)
  })
  it('does not run a queued transition after the session stops', async () => {
    let finish!: () => void
    const write = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          finish = resolve
        }),
    )
    const { flow, effects } = setup({ writeInput: write })
    const first = flow.observe({ from: 'mac', to: 'windows' })
    const second = flow.observe({ from: 'windows', to: 'mac' })
    await Promise.resolve()
    flow.stop()
    finish()
    await Promise.all([first, second])
    expect(write).toHaveBeenCalledTimes(1)
    expect(effects.finished).not.toHaveBeenCalled()
  })
})
