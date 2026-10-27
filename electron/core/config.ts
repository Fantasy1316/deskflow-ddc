import { configSchema } from '../../src/shared/config.js'
import type { Config } from '../../src/shared/types.js'
export { configSchema }

export function defaults(name: string, platform = process.platform): Config {
  return {
    deviceName: name,
    role: 'host',
    peerAddress: '',
    controlPort: 49728,
    deskflowPort: 24800,
    deskflowPath: '',
    displayId: '',
    localInput: null,
    peerInput: null,
    shortcut: platform === 'win32' ? 'control+alt+F10' : 'control+alt+F12',
    clipboard: true,
    clipboardMegabytes: 4,
    layoutSide: 'off',
    crossScreenVideo: 'follow',
    retainedDisplayId: '',
  }
}
export function requireSupportedShortcut(c: Config, platform = process.platform) {
  if (platform === 'win32' && c.role === 'host' && /\+F12$/.test(c.shortcut)) {
    throw new Error(
      'F12 是 Windows 保留键，无法用作 Deskflow 切换快捷键。请在“共享设置”选择 Ctrl + Alt + F10 或 F9，保存后重新开始共享。',
    )
  }
}
export function readyForDisplay(c: Config) {
  return (
    !!c.displayId && c.localInput !== null && c.peerInput !== null && c.localInput !== c.peerInput
  )
}
export function requireReady(c: Config) {
  configSchema.parse(c)
  if (!readyForDisplay(c)) throw new Error('请先选择显示器，并填写本机和另一台电脑的输入源数值')
}
