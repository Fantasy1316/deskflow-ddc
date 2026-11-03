import { createSSRApp, h } from 'vue'
import { renderToString } from 'vue/server-renderer'
import { describe, expect, it } from 'vitest'
import LinuxSupport from '../src/components/LinuxSupport.vue'
import { platformLabel } from '../src/shared/workspace'
import type { Snapshot } from '../src/shared/types'

describe('Ubuntu compatibility', () => {
  it('labels Linux separately from Windows and browser preview', () => {
    expect(platformLabel('linux')).toBe('Ubuntu / Linux')
    expect(platformLabel('win32')).toBe('Windows')
    expect(platformLabel('browser')).toBe('浏览器预览')
  })
  it('shows session and DDC setup guidance without claiming hardware support', async () => {
    const state = {
      platform: 'linux',
      dependencies: { accessibility: false, displayControl: true },
    } as Snapshot
    const html = await renderToString(createSSRApp(() => h(LinuxSupport, { state, busy: false })))
    expect(html).toContain('Ubuntu on Xorg')
    expect(html).toContain('ddcutil')
    expect(html).toContain('当前会话或输入桥不可用')
    expect(html).toContain('无需以 root 运行应用')
  })
  it('does not show Ubuntu instructions on other systems', async () => {
    const state = { platform: 'darwin' } as Snapshot
    const html = await renderToString(createSSRApp(() => h(LinuxSupport, { state, busy: false })))
    expect(html).not.toContain('Ubuntu on Xorg')
  })
})
