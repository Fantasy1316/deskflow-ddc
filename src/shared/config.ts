import { z } from 'zod'
import type { Config } from './types.js'

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
      displayMethod: z.enum(['ddc', 'signal']).default('ddc'),
      outputId: z
        .string()
        .max(128)
        .refine((v) => !/[\r\n\0]/.test(v))
        .default(''),
      mode: z.enum(['sharing', 'display-only']).default('sharing'),
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
      fallbackIps: z.array(z.string().ip()).max(8).optional(),
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
      modifierMapping: z.enum(['original', 'common']).default('original'),
      windowsSecureInput: z.boolean().default(false),
      clipboard: z.boolean(),
      clipboardMegabytes: z.number().int().min(1).max(32),
      layoutSide: z.enum(['off', 'left', 'right', 'top', 'bottom']).default('off'),
      crossScreenVideo: z.enum(['manual', 'follow', 'keep']).default('manual'),
      retainedDisplayId: z
        .string()
        .max(128)
        .regex(/^[a-zA-Z0-9_-]*$/)
        .default(''),
    })
    .strict()
    .superRefine((c, ctx) => {
      if (c.displayMethod === 'signal' && (c.mode !== 'sharing' || c.crossScreenVideo !== 'manual'))
        ctx.addIssue({
          code: 'custom',
          message: '自动选源兜底需要联网共享，画面仅由快捷键或按钮切换',
        })
      if (c.controlPort === c.deskflowPort)
        ctx.addIssue({ code: 'custom', message: '控制端口和键鼠端口必须不同' })
      if (c.displayMethod === 'ddc' && c.localInput !== null && c.localInput === c.peerInput)
        ctx.addIssue({ code: 'custom', message: '两台电脑的输入源数值必须不同' })
    }),
)

/** Frontend-only initial state; the desktop backend supplies persisted settings. */
export function defaultConfig(deviceName: string): Config {
  return {
    displayMethod: 'ddc',
    outputId: '',
    mode: 'sharing',
    deviceName,
    role: 'host',
    peerAddress: '',
    fallbackIps: [],
    controlPort: 49729,
    deskflowPort: 24801,
    deskflowPath: '',
    displayId: '',
    localInput: null,
    peerInput: null,
    shortcut: 'control+alt+F10',
    modifierMapping: 'original',
    windowsSecureInput: false,
    clipboard: true,
    clipboardMegabytes: 4,
    layoutSide: 'off',
    crossScreenVideo: 'manual',
    retainedDisplayId: '',
  }
}
