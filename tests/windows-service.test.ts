import { createSSRApp, h } from 'vue'
import { renderToString } from 'vue/server-renderer'
import { expect, it } from 'vitest'
import WindowsServiceSettings from '../src/components/WindowsServiceSettings.vue'
import { configSchema, defaultConfig } from '../src/shared/config'
import { actionSchema } from '../src/shared/actions'
import type { Snapshot } from '../src/shared/types'
async function render(
  platform: string,
  installed: boolean,
  current: boolean,
  running = false,
  enabled = false,
) {
  return renderToString(
    createSSRApp({
      render: () =>
        h(WindowsServiceSettings, {
          state: { platform, running, windowsService: { installed, current } } as Snapshot,
          busy: false,
          modelValue: enabled,
        }),
    }),
  )
}
it('keeps secure desktop opt-in disabled for legacy and new configurations', () => {
  const { windowsSecureInput: _, ...legacy } = defaultConfig('desktop')
  expect(configSchema.parse(legacy).windowsSecureInput).toBe(false)
  expect(configSchema.parse({ ...legacy, windowsSecureInput: true }).windowsSecureInput).toBe(true)
  expect(configSchema.safeParse({ ...legacy, windowsSecureInput: 'yes' }).success).toBe(false)
})
it('offers Windows installation and prevents enabling an absent or outdated service', async () => {
  for (const installed of [false, true]) {
    const html = await render('win32', installed, false)
    expect(html).toContain(installed ? '需要更新' : '未安装')
    expect(html).toMatch(/<input[^>]*disabled/)
  }
  expect(await render('darwin', false, false)).not.toContain('安装服务')
  const ready = await render('win32', true, true)
  expect(ready).toContain('服务就绪')
  expect(ready).not.toMatch(/<input[^>]*disabled/)
  const sharing = await render('win32', true, true, true)
  expect(sharing).toMatch(/<input[^>]*disabled/)
  expect(sharing).toMatch(/<button[^>]*disabled/)
  // An unavailable/removed service must never trap an already enabled setting.
  expect(await render('win32', false, false, false, true)).not.toMatch(/<input[^>]*disabled/)
})
it('exposes only explicit lock and service actions without command execution arguments', () => {
  for (const type of [
    'lockPeer',
    'installWindowsService',
    'removeWindowsService',
    'checkWindowsService',
  ]) {
    expect(actionSchema.parse({ type })).toEqual({ type })
    expect(actionSchema.safeParse({ type, command: 'arbitrary' }).success).toBe(false)
  }
})
