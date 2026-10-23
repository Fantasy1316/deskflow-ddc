import { execFile } from 'node:child_process'
import { access } from 'node:fs/promises'
import { join } from 'node:path'
import type { Monitor } from '../../src/shared/types.js'
import { WindowsHelper } from './windows-helper.js'

export function parseMacDisplays(output: string): Monitor[] {
  return [...output.matchAll(/^\[(\d+)\][ \t]*(.*?)\s*\(([A-Fa-f0-9-]{36})\)[ \t]*$/gm)].map(
    (match) => ({ id: match[3]!, name: match[2]!.trim(), readable: false }),
  )
}

export function run(file: string, args: string[], timeout = 7000): Promise<string> {
  return new Promise((resolve, reject) =>
    execFile(
      file,
      args,
      { timeout, maxBuffer: 1024 * 1024, windowsHide: true },
      (error, stdout, stderr) => {
        if (error)
          reject(
            new Error((stderr?.trim() || stdout?.trim() || error.message).slice(0, 700), {
              cause: error,
            }),
          )
        else resolve(stdout.trim())
      },
    ),
  )
}
export class DisplayController {
  private helper?: WindowsHelper
  constructor(
    private resources: string,
    private report?: (event: string, details: unknown) => void,
  ) {}
  private get macBinary() {
    return join(this.resources, 'bin/m1ddc')
  }
  private get windowsScript() {
    return join(this.resources, 'monitor.ps1')
  }
  private async win(
    action: 'list' | 'set' | 'host' | 'peer' | 'pointer',
    id = '',
    value = 0,
    point?: { x: number; y: number },
  ) {
    this.helper ??= new WindowsHelper(this.windowsScript)
    return this.helper.request(action, { monitorId: id, value, x: point?.x, y: point?.y })
  }
  async dispose() {
    await this.helper?.close()
  }
  async available() {
    try {
      await access(
        process.platform === 'darwin' && process.arch === 'arm64'
          ? this.macBinary
          : process.platform === 'win32'
            ? this.windowsScript
            : '/screenbridge-unsupported',
      )
      return true
    } catch {
      return false
    }
  }
  async list(): Promise<Monitor[]> {
    if (process.platform === 'win32') {
      const data = JSON.parse(await this.win('list')) as Monitor | Monitor[] | null
      return data ? (Array.isArray(data) ? data : [data]) : []
    }
    if (process.platform !== 'darwin' || process.arch !== 'arm64')
      throw new Error('当前原生显示器后端支持 Apple Silicon macOS 和 Windows')
    const output = await run(this.macBinary, ['display', 'list'])
    return Promise.all(
      parseMacDisplays(output).map(async (monitor) => {
        try {
          const value = await run(this.macBinary, ['display', monitor.id, 'get', 'input'], 2500)
          if (/^\d+$/.test(value)) {
            monitor.currentInput = Number(value)
            monitor.readable = true
          }
        } catch (error) {
          this.report?.('display.readFailed', { monitorId: monitor.id, error })
        }
        return monitor
      }),
    )
  }
  async setInput(displayId: string, value: number) {
    if (!displayId || !Number.isInteger(value) || value < 0 || value > 65535)
      throw new Error('显示器或输入源数值无效')
    this.report?.('display.writeRequested', { displayId, value })
    try {
      if (process.platform === 'win32') await this.win('set', displayId, value)
      else if (process.platform === 'darwin' && process.arch === 'arm64')
        await run(this.macBinary, ['display', displayId, 'set', 'input', String(value)])
      else throw new Error('当前系统不支持原生 DDC 控制')
      this.report?.('display.writeCommandCompleted', { displayId, value })
    } catch (error) {
      this.report?.('display.writeFailed', { displayId, value, error })
      throw error
    }
  }
  async injectTarget(target: 'host' | 'peer') {
    this.report?.('display.targetCommandRequested', { target })
    try {
      if (process.platform === 'win32') await this.win(target)
      else if (process.platform === 'darwin')
        await run(join(this.resources, 'bin/mac-hotkey'), [target])
      else throw new Error('当前系统不支持快捷键注入')
      this.report?.('display.targetCommandCompleted', { target })
    } catch (error) {
      this.report?.('display.targetCommandFailed', { target, error })
      throw error
    }
  }
  async checkAccessibility() {
    if (process.platform === 'win32') return true
    try {
      return (await run(join(this.resources, 'bin/mac-hotkey'), ['check'])) === 'true'
    } catch (error) {
      this.report?.('display.accessibilityCheckFailed', { error })
      return false
    }
  }
  async landOnScreen(point: { x: number; y: number }) {
    if (![point.x, point.y].every(Number.isFinite)) throw new Error('鼠标落点无效')
    this.report?.('pointer.landRequested', {})
    try {
      if (process.platform === 'win32')
        await this.win('pointer', '', 0, { x: Math.round(point.x), y: Math.round(point.y) })
      else if (process.platform === 'darwin')
        await run(join(this.resources, 'bin/mac-hotkey'), [
          'pointer',
          String(point.x),
          String(point.y),
        ])
      else throw new Error('当前系统不支持鼠标定位')
      this.report?.('pointer.landCompleted', {})
    } catch (error) {
      this.report?.('pointer.landFailed', { error })
      throw error
    }
  }
}
