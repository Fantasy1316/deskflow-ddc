import { describe, expect, it } from 'vitest'
import { configSchema, defaultConfig } from '../src/shared/config'
import { actionSchema } from '../src/shared/actions'

describe('desktop frontend configuration', () => {
  it('starts with manual video switching and the 0.2 ports', () => {
    const config = defaultConfig('desktop')
    expect(configSchema.parse(config)).toEqual(config)
    expect(config.crossScreenVideo).toBe('manual')
    expect([config.controlPort, config.deskflowPort]).toEqual([49729, 24801])
    expect(config.shortcut).toBe('control+alt+F10')
    const { crossScreenVideo: _, ...withoutPolicy } = config
    expect(configSchema.parse(withoutPolicy).crossScreenVideo).toBe('manual')
  })

  it('does not share mutable defaults between drafts', () => {
    const first = defaultConfig('first')
    first.fallbackIps!.push('192.168.1.2')
    expect(defaultConfig('second').fallbackIps).toEqual([])
  })

  it('keeps legacy modifiers unchanged and accepts only supported mapping modes', () => {
    const config = defaultConfig('desktop')
    const { modifierMapping: _, ...legacy } = config
    expect(config.modifierMapping).toBe('original')
    expect(configSchema.parse(legacy).modifierMapping).toBe('original')
    const action = actionSchema.parse({
      type: 'save',
      config: { ...config, modifierMapping: 'common' },
    })
    expect(action.type === 'save' && action.config.modifierMapping).toBe('common')
    expect(configSchema.safeParse({ ...config, modifierMapping: 'invalid' }).success).toBe(false)
  })

  it('rejects conflicting ports and identical video inputs', () => {
    const config = defaultConfig('desktop')
    expect(configSchema.safeParse({ ...config, controlPort: config.deskflowPort }).success).toBe(
      false,
    )
    expect(configSchema.safeParse({ ...config, localInput: 1, peerInput: 1 }).success).toBe(false)
  })

  it('validates addresses and backup IPs before sending settings', () => {
    const config = defaultConfig('desktop')
    expect(configSchema.safeParse({ ...config, peerAddress: 'host\ninvalid' }).success).toBe(false)
    expect(configSchema.safeParse({ ...config, fallbackIps: ['not-an-ip'] }).success).toBe(false)
    expect(configSchema.safeParse({ ...config, fallbackIps: ['192.168.1.2', '::1'] }).success).toBe(
      true,
    )
  })

  it('validates actions and rejects extra fields', () => {
    expect(actionSchema.parse({ type: 'switch', target: 'peer' })).toEqual({
      type: 'switch',
      target: 'peer',
    })
    for (const action of [
      { type: 'switch', target: 'unknown' },
      { type: 'testInput', value: -1 },
      { type: 'stop', command: 'unexpected' },
      { type: 'unknown' },
    ])
      expect(actionSchema.safeParse(action).success).toBe(false)
  })
})
