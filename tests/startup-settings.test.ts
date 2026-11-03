import { createSSRApp, h } from 'vue'
import { renderToString } from 'vue/server-renderer'
import { describe, expect, it } from 'vitest'
import StartupSettings from '../src/components/StartupSettings.vue'
import { actionSchema } from '../src/shared/actions'
import { defaultConfig } from '../src/shared/config'
import type { Snapshot } from '../src/shared/types'

function render(changes: Partial<Snapshot> = {}, busy = false) {
  const state = { config: defaultConfig('desktop'), ...changes } as Snapshot
  return renderToString(createSSRApp(() => h(StartupSettings, { state, busy })))
}

describe('launch preferences', () => {
  it('defaults both options off and allows editing while sharing in either role', async () => {
    for (const role of ['host', 'client'] as const) {
      const html = await render({ running: true, config: { ...defaultConfig('desktop'), role } })
      expect(html).toContain('开机自启动')
      expect(html).toContain('启动后自动共享')
      expect(html).toContain('手动停止后，本次运行保持停止')
      expect(html).not.toMatch(/<input[^>]*(checked|disabled)/)
    }
  })

  it('shows an OS mismatch with a save action and locks toggles during an action', async () => {
    const state = {
      startup: { launchAtLogin: true, autoShare: false },
      startupRegistered: false,
    }
    const html = await render(state)
    expect(html).toContain('系统登录启动项与保存的设置不一致')
    expect(html).not.toMatch(/<button[^>]*type="submit"[^>]*disabled/)
    const locked = await render(state, true)
    expect(locked.match(/<input[^>]*disabled/g)).toHaveLength(2)
  })

  it('explains independent display control without requiring a network peer', async () => {
    const html = await render({ config: { ...defaultConfig('desktop'), mode: 'display-only' } })
    expect(html).toContain('启动后自动启用显示器控制')
    expect(html).toContain('不建立网络连接')
  })

  it('sends only the separate startup preferences and rejects invalid values', () => {
    for (const launchAtLogin of [true, false]) {
      for (const autoShare of [true, false]) {
        const action = { type: 'saveStartup', settings: { launchAtLogin, autoShare } }
        expect(actionSchema.parse(action)).toEqual(action)
      }
    }
    expect(
      actionSchema.safeParse({
        type: 'saveStartup',
        settings: { launchAtLogin: 'yes', autoShare: true },
      }).success,
    ).toBe(false)
    expect(
      actionSchema.safeParse({
        type: 'saveStartup',
        settings: { launchAtLogin: true, autoShare: true, command: 'unexpected' },
      }).success,
    ).toBe(false)
    expect(actionSchema.parse({ type: 'checkStartup' })).toEqual({ type: 'checkStartup' })
  })
})
