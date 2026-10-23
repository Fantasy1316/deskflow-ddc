import { describe, expect, it } from 'vitest'
import {
  configSchema,
  defaults,
  readyForDisplay,
  requireSupportedShortcut,
} from '../electron/core/config'
import { makeServerConfig, makeSettings, parseSwitch } from '../electron/core/deskflow-config'
const a = '72e012e5-832f-4a18-9f30-bc71355d2c15',
  b = '2ab157ff-4d51-452b-9010-9b41ae75557c'
describe('user-selected device and input configuration', () => {
  it('keeps missing inputs distinct from explicitly configured zero', () => {
    const config = defaults('Mac')
    expect(readyForDisplay(config)).toBe(false)
    Object.assign(config, { displayId: 'test', localInput: 0, peerInput: 3 })
    expect(configSchema.parse(config).localInput).toBe(0)
    expect(readyForDisplay(config)).toBe(true)
    expect(configSchema.safeParse({ ...config, peerInput: 0 }).success).toBe(false)
    expect(configSchema.safeParse({ ...config, localInput: null }).success).toBe(true)
  })
  it('rejects configuration injection, invalid port collisions and reserved shortcuts', () => {
    const config = defaults('Windows')
    expect(configSchema.safeParse({ ...config, peerAddress: 'host\n[security]' }).success).toBe(
      false,
    )
    expect(configSchema.safeParse({ ...config, controlPort: 24800 }).success).toBe(false)
    expect(configSchema.safeParse({ ...config, shortcut: 'control+alt+shift+F18' }).success).toBe(
      false,
    )
  })
  it('generates a host configuration independent of operating system and without screen edges', () => {
    const text = makeServerConfig(a, b, 'control+alt+F12')
    expect(text).toContain(`switchToScreen(sb-${a})`)
    expect(text).toContain(`switchToScreen(sb-${b})`)
    expect(text).toContain('= switchToNextScreen\n')
    expect(text).not.toContain('switchToNextScreen()')
    expect(text).not.toMatch(/right\s*=/)
    const settings = makeSettings(
      { ...defaults('Windows'), role: 'host', clipboard: false },
      a,
      b,
      'C:/Deskflow DDC/engine',
      'a'.repeat(64),
      '',
    )
    expect(settings.text).toContain('coreMode=2')
    expect(settings.text).toContain('tlsEnabled=true')
    expect(settings.text).toContain('checkPeerFingerprints=true')
    expect(settings.text).toContain('enableClipboard=false')
    expect(settings.trust).toBe(`v2:sha256:${'a'.repeat(64)}\n`)
  })
  it('recognizes actual Deskflow switch logs and ignores other lines', () => {
    expect(parseSwitch(`INFO: switch from "sb-${a}" to "sb-${b}" at 100,200`)).toEqual({
      from: a,
      to: b,
    })
    expect(parseSwitch('INFO: accepted secure socket')).toBeNull()
  })
  it('matches the Mac Option injection against Deskflow 1.27 hotkey registration', () => {
    const mac = makeServerConfig(a, b, 'control+alt+F12', 'darwin')
    for (const chord of [
      'control+super+F12',
      'control+super+Escape',
      'control+super+shift+F18',
      'control+super+shift+F19',
    ])
      expect(mac).toContain(`keystroke(${chord})`)
    expect(mac).not.toContain('keystroke(control+alt+')
    const win = makeServerConfig(a, b, 'control+alt+F12', 'win32')
    for (const chord of [
      'control+alt+F12',
      'control+alt+Escape',
      'control+alt+shift+F18',
      'control+alt+shift+F19',
    ])
      expect(win).toContain(`keystroke(${chord})`)
    expect(win).not.toContain('super')
  })
  it('migrates existing configurations with cross-screen mode disabled', () => {
    const { layoutSide, crossScreenVideo, retainedDisplayId, ...old } = defaults('Mac')
    expect(configSchema.parse(old)).toMatchObject({
      layoutSide: 'off',
      crossScreenVideo: 'follow',
      retainedDisplayId: '',
    })
    expect(configSchema.parse({ ...old, internalScreenSide: 'right' })).toMatchObject({
      layoutSide: 'right',
      crossScreenVideo: 'keep',
      retainedDisplayId: 'internal',
    })
    expect(configSchema.parse({ ...old, internalScreenSide: 'off' })).toMatchObject({
      layoutSide: 'off',
      crossScreenVideo: 'follow',
      retainedDisplayId: '',
    })
  })
  it.each([
    ['left', 'left', 'right'],
    ['right', 'right', 'left'],
    ['top', 'up', 'down'],
    ['bottom', 'down', 'up'],
  ] as const)(
    'links both directions with the host on the %s using Deskflow direction names',
    (side, inbound, outbound) => {
      const text = makeServerConfig(a, b, 'control+alt+F12', 'darwin', side, 'keep')
      expect(text).toContain(`sb-${b}:\n\t\t${inbound} = sb-${a}`)
      expect(text).toContain(`sb-${a}:\n\t\t${outbound} = sb-${b}`)
      expect(text).not.toMatch(/\b(top|bottom) =/)
      // Electron owns the public video-switch shortcut in this mode; Deskflow
      // owns keyboard-only recovery and explicit F18/F19 target injection.
      expect(text).not.toContain('keystroke(control+super+F12)')
      expect(text).toContain('keystroke(control+super+shift+F19)')
    },
  )
  it.each(['darwin', 'win32'] as const)(
    'enables native edges and the public video shortcut on %s without an independent screen',
    (platform) => {
      const text = makeServerConfig(a, b, 'control+alt+F10', platform, 'left', 'follow')
      expect(text).toContain(`sb-${a}:\n\t\tright = sb-${b}`)
      expect(text).toContain(`sb-${b}:\n\t\tleft = sb-${a}`)
      expect(text).toContain('= switchToNextScreen\n')
      const separate = makeServerConfig(a, b, 'control+alt+F10', platform, 'left', 'keep')
      expect(separate).not.toContain('= switchToNextScreen')
    },
  )
  it('avoids Windows reserved F12 without changing a Mac host or receiver setting', () => {
    expect(defaults('Windows', 'win32').shortcut).toBe('control+alt+F10')
    const mac = defaults('Mac', 'darwin')
    expect(mac.shortcut).toBe('control+alt+F12')
    expect(() => requireSupportedShortcut(mac, 'darwin')).not.toThrow()
    expect(() => requireSupportedShortcut(mac, 'win32')).toThrow('F12 是 Windows 保留键')
    expect(() =>
      requireSupportedShortcut({ ...mac, shortcut: 'control+alt+shift+F12' }, 'win32'),
    ).toThrow('F12 是 Windows 保留键')
    expect(() => requireSupportedShortcut({ ...mac, role: 'client' }, 'win32')).not.toThrow()
    expect(() =>
      requireSupportedShortcut({ ...mac, shortcut: 'control+alt+F10' }, 'win32'),
    ).not.toThrow()
  })
})
