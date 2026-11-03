import { createSSRApp, h } from 'vue'
import { renderToString } from 'vue/server-renderer'
import { describe, expect, it } from 'vitest'
import ModifierSettings from '../src/components/ModifierSettings.vue'

describe('cross-system modifier settings', () => {
  it('shows an editable provider mode and explains the full modifier swap', async () => {
    const html = await renderToString(
      createSSRApp(() =>
        h(ModifierSettings, {
          modelValue: 'common',
          isHost: true,
          locked: false,
          connected: false,
        }),
      ),
    )
    expect(html).toMatch(/<option value="common" selected>/)
    expect(html).not.toMatch(/<select[^>]*disabled/)
    expect(html).toContain('Control → Win')
    expect(html).toContain('Win → Control')
  })

  it('shows the provider mode read-only on the receiver instead of its local draft', async () => {
    const html = await renderToString(
      createSSRApp(() =>
        h(ModifierSettings, {
          modelValue: 'original',
          hostMapping: 'common',
          isHost: false,
          locked: false,
          connected: true,
        }),
      ),
    )
    expect(html).toMatch(/<option value="common" selected>/)
    expect(html).toMatch(/<select[^>]*disabled/)
    expect(html).toContain('下方显示提供端的设置')
  })

  it('locks changes during sharing', async () => {
    const html = await renderToString(
      createSSRApp(() =>
        h(ModifierSettings, {
          modelValue: 'original',
          isHost: true,
          locked: true,
          connected: true,
        }),
      ),
    )
    expect(html).toMatch(/<option value="original" selected>/)
    expect(html).toMatch(/<select[^>]*disabled/)
  })
})
