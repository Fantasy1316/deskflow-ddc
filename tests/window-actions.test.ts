import { describe, expect, it, vi } from 'vitest'
import { assertRendererSender, closeMainWindow } from '../electron/core/window-lifecycle'
import { actionSchema } from '../src/shared/actions'
import { defaults } from '../electron/core/config'

describe('window shutdown and renderer boundary', () => {
  it('requests graceful quit instead of destroying a running window without a tray', () => {
    const event = { preventDefault: vi.fn() },
      hide = vi.fn(),
      quit = vi.fn()
    closeMainWindow({ quitting: false, running: true, hasTray: false }, event, hide, quit)
    expect(event.preventDefault).toHaveBeenCalledOnce()
    expect(quit).toHaveBeenCalledOnce()
    expect(hide).not.toHaveBeenCalled()
    // The subsequent close after before-quit cleanup must be allowed.
    event.preventDefault.mockClear()
    closeMainWindow({ quitting: true, running: false, hasTray: false }, event, hide, quit)
    expect(event.preventDefault).not.toHaveBeenCalled()
  })
  it('keeps active sharing reachable through a tray', () => {
    const hide = vi.fn(),
      quit = vi.fn()
    closeMainWindow(
      { quitting: false, running: true, hasTray: true },
      { preventDefault() {} },
      hide,
      quit,
    )
    expect(hide).toHaveBeenCalledOnce()
    expect(quit).not.toHaveBeenCalled()
  })
  it('accepts only the living window main frame', () => {
    const frame = {},
      window = { isDestroyed: () => false, webContents: { mainFrame: frame } }
    expect(() =>
      assertRendererSender({ sender: window.webContents, senderFrame: frame }, window),
    ).not.toThrow()
    for (const event of [
      { sender: {}, senderFrame: frame },
      { sender: window.webContents, senderFrame: {} },
    ]) {
      expect(() => assertRendererSender(event, window)).toThrow('无效')
    }
    expect(() =>
      assertRendererSender(
        { sender: window.webContents, senderFrame: frame },
        { ...window, isDestroyed: () => true },
      ),
    ).toThrow()
  })
  it('validates action payloads as a complete discriminated union', () => {
    expect(actionSchema.parse({ type: 'save', config: defaults('test') }).type).toBe('save')
    for (const invalid of [
      { type: 'save' },
      { type: 'testInput', value: -1 },
      { type: 'switch', target: 'other' },
      { type: 'stop', extra: true },
      { type: 'new-action' },
    ]) {
      expect(actionSchema.safeParse(invalid).success).toBe(false)
    }
  })
})
