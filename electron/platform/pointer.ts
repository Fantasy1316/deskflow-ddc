import { globalShortcut, screen } from 'electron'
import type { Runtime } from '../core/runtime.js'
import type { ScreenSide } from '../../src/shared/types.js'
import { contains, crossesSharedEdge, type Point } from '../core/pointer-geometry.js'

const opposite: Record<ScreenSide, ScreenSide> = {
  left: 'right',
  right: 'left',
  top: 'bottom',
  bottom: 'top',
}
/** Samples cursor position only while a selected independent screen is in use. */
export class PointerBridge {
  private timer?: NodeJS.Timeout
  private previous?: Point
  private cooldown = 0
  private moving = false
  private accelerator = ''
  private wantedAccelerator = ''
  private shortcutRetry?: NodeJS.Timeout
  private shortcutDelay = 1000
  private registering = false
  private epoch = 0
  constructor(private runtime: Runtime) {
    this.refreshDisplays()
    runtime.on('state', () => this.sync())
    runtime.on('control-state', () => this.sync())
    const displaysChanged = () => {
      this.refreshDisplays()
      if (
        runtime.state.running &&
        runtime.state.config.retainedDisplayId &&
        !this.hasRetainedDisplay()
      ) {
        runtime.log('error', '所选独立屏幕已断开，请重新选择屏幕后开始共享')
        void runtime
          .stop()
          .catch((error) => runtime.log('error', '停止共享失败，请查看日志', error))
      }
      runtime.emit('state', runtime.snapshot())
    }
    screen.on('display-added', displaysChanged)
    screen.on('display-removed', displaysChanged)
    screen.on('display-metrics-changed', displaysChanged)
  }
  private refreshDisplays() {
    this.runtime.state.cursorDisplays = screen.getAllDisplays().map((display) => ({
      id: String(display.id),
      name: display.label || (display.internal ? '内置屏幕' : `屏幕 ${display.id}`),
      internal: display.internal,
      bounds: display.bounds,
    }))
  }
  private selectedDisplay() {
    const id = this.runtime.state.config.retainedDisplayId
    return screen
      .getAllDisplays()
      .find((display) => (id === 'internal' ? display.internal : String(display.id) === id))
  }
  hasRetainedDisplay() {
    return !!this.selectedDisplay()
  }
  validate() {
    this.refreshDisplays()
    if (!this.hasRetainedDisplay())
      throw new Error('未发现所选独立屏幕，请在共享设置中重新选择，或选择“没有独立屏幕”')
  }
  private get side(): ScreenSide | null {
    const side = this.runtime.layout?.hostSide
    if (!side || side === 'off') return null
    return this.runtime.state.config.role === 'host' ? side : opposite[side]
  }
  private sync() {
    const state = this.runtime.state
    const enabled = this.runtime.pointerMode && state.connected && state.engine === 'ready'
    const wanted =
      enabled && state.config.role === 'host' && this.runtime.layout?.video === 'keep'
        ? state.config.shortcut
            .replace('control', 'Control')
            .replace('alt', 'Alt')
            .replace('shift', 'Shift')
        : ''
    if (wanted !== this.wantedAccelerator) {
      clearTimeout(this.shortcutRetry)
      this.shortcutRetry = undefined
      if (this.accelerator) globalShortcut.unregister(this.accelerator)
      this.accelerator = ''
      this.wantedAccelerator = wanted
      this.shortcutDelay = 1000
    }
    if (wanted && !this.accelerator && !this.shortcutRetry && !this.registering)
      this.registerShortcut()
    const sample = enabled && this.runtime.layout?.video === 'keep' && this.hasRetainedDisplay()
    if (sample && !this.timer) {
      ++this.epoch
      this.previous = undefined
      this.timer = setInterval(() => this.poll(), 16)
    } else if (!sample && this.timer) {
      ++this.epoch
      clearInterval(this.timer)
      this.timer = undefined
      this.previous = undefined
    }
  }
  private registerShortcut() {
    const wanted = this.wantedAccelerator
    if (!wanted) return
    this.registering = true
    let registered = false
    try {
      registered = globalShortcut.register(wanted, () => {
        void this.runtime
          .toggleDisplay()
          .catch((error) => this.runtime.log('error', error.message, error))
      })
    } catch {
      /* Invalid/unavailable system registration follows the same retry path. */
    } finally {
      this.registering = false
    }
    if (registered) {
      this.accelerator = wanted
      this.shortcutDelay = 1000
      return
    }
    // Set the retry before logging: log publishes state synchronously.
    this.shortcutRetry = setTimeout(() => {
      this.shortcutRetry = undefined
      this.sync()
    }, this.shortcutDelay)
    this.shortcutRetry.unref?.()
    if (this.shortcutDelay === 1000)
      this.runtime.log(
        'error',
        '切换快捷键被其他程序占用，将自动重试；也可使用 Deskflow DDC 按钮或停止共享后更换快捷键',
      )
    this.shortcutDelay = Math.min(this.shortcutDelay * 2, 30000)
  }
  private poll() {
    const state = this.runtime.state
    if (
      this.moving ||
      Date.now() < this.cooldown ||
      state.switching ||
      state.activeDeviceId !== state.deviceId ||
      state.displayDeviceId !== state.peer?.id
    ) {
      this.previous = undefined
      return
    }
    const selected = this.selectedDisplay(),
      side = this.side
    if (!selected || !side) {
      this.previous = undefined
      return
    }
    const point = screen.getCursorScreenPoint(),
      previous = this.previous
    this.previous = point
    if (previous && crossesSharedEdge(previous, point, selected.bounds, side)) {
      this.previous = undefined
      this.cooldown = Date.now() + 600
      void this.runtime
        .switchKeyboardToPeer()
        .catch((error) => this.runtime.log('error', error.message, error))
    } else if (!contains(selected.bounds, point)) {
      // The OS can retain the inactive shared desktop in its aggregate bounds.
      void this.land().catch((error) => this.runtime.log('error', error.message, error))
    }
  }
  async land() {
    if (this.moving) return
    const side = this.side
    if (!side || !this.hasRetainedDisplay()) throw new Error('所选独立屏幕不可用')
    const epoch = this.epoch
    this.moving = true
    this.previous = undefined
    this.cooldown = Date.now() + 600
    try {
      // Deskflow emits the switch log before completing its native cursor warp.
      await new Promise((resolve) => setTimeout(resolve, 80))
      if (
        epoch !== this.epoch ||
        !this.runtime.pointerMode ||
        !this.runtime.state.connected ||
        this.runtime.state.engine !== 'ready'
      )
        return
      const display = this.selectedDisplay()
      if (!display) throw new Error('所选独立屏幕已断开')
      const b = display.bounds,
        current = screen.getCursorScreenPoint()
      const margin = Math.min(24, b.width / 4, b.height / 4)
      const point = {
        x: Math.max(b.x + margin, Math.min(current.x, b.x + b.width - margin)),
        y: Math.max(b.y + margin, Math.min(current.y, b.y + b.height - margin)),
      }
      if (side === 'left') point.x = b.x + b.width - margin
      else if (side === 'right') point.x = b.x + margin
      else if (side === 'top') point.y = b.y + b.height - margin
      else point.y = b.y + margin
      // Windows SetCursorPos uses physical pixels; Electron bounds use DIPs.
      const target =
        this.runtime.state.platform === 'win32' ? screen.dipToScreenPoint(point) : point
      await this.runtime.display.landOnScreen(target)
    } finally {
      this.moving = false
    }
  }
}
