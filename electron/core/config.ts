import { z } from 'zod'
import type { Config } from '../../src/shared/types.js'

// Migrate the earlier Mac-only setting without changing its direction or behavior.
export const configSchema = z.preprocess(
  (raw) => {
    if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return raw
    const { internalScreenSide, ...config } = raw as Record<string, unknown>
    if (internalScreenSide !== undefined && config.layoutSide === undefined) {
      config.layoutSide = internalScreenSide
      if (internalScreenSide !== 'off') {
        config.crossScreenVideo ??= 'keep'
        config.retainedDisplayId ??= 'internal'
      }
    }
    return config
  },
  z
    .object({
      deviceName: z
        .string()
        .trim()
        .min(1)
        .max(48)
        .refine((v) => !/[\r\n\0]/.test(v)),
      role: z.enum(['host', 'client']),
      peerAddress: z
        .string()
        .trim()
        .max(253)
        .refine((v) => !/[\r\n\0\s]/.test(v)),
      controlPort: z.number().int().min(1024).max(65535),
      deskflowPort: z.number().int().min(1024).max(65535),
      deskflowPath: z
        .string()
        .max(4096)
        .refine((v) => !/[\r\n\0]/.test(v)),
      displayId: z
        .string()
        .max(512)
        .refine((v) => !/[\r\n\0]/.test(v)),
      localInput: z.number().int().min(0).max(65535).nullable(),
      peerInput: z.number().int().min(0).max(65535).nullable(),
      shortcut: z.string().regex(/^(control\+alt\+(shift\+)?)(F(?:[1-9]|1[0-7]))$/),
      clipboard: z.boolean(),
      clipboardMegabytes: z.number().int().min(1).max(32),
      layoutSide: z.enum(['off', 'left', 'right', 'top', 'bottom']).default('off'),
      crossScreenVideo: z.enum(['follow', 'keep']).default('follow'),
      retainedDisplayId: z
        .string()
        .max(128)
        .regex(/^[a-zA-Z0-9_-]*$/)
        .default(''),
    })
    .strict()
    .superRefine((c, ctx) => {
      if (c.controlPort === c.deskflowPort)
        ctx.addIssue({ code: 'custom', message: '控制端口和键鼠端口必须不同' })
      if (c.localInput !== null && c.localInput === c.peerInput)
        ctx.addIssue({ code: 'custom', message: '两台电脑的输入源数值必须不同' })
    }),
)
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
