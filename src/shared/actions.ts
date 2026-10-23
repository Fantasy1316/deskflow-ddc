import { z } from 'zod'
import { configSchema } from '../../electron/core/config.js'

export const actionSchema = z.discriminatedUnion('type', [
  z.object({ type: z.literal('save'), config: configSchema }).strict(),
  z.object({ type: z.literal('start'), pairingCode: z.string().max(128).optional() }).strict(),
  z.object({ type: z.literal('switch'), target: z.enum(['local', 'peer']) }).strict(),
  z.object({ type: z.literal('testInput'), value: z.number().int().min(0).max(65535) }).strict(),
  ...(
    [
      'scan',
      'stop',
      'chooseDeskflow',
      'rotatePairing',
      'copyPairing',
      'openAccessibility',
      'openDataFolder',
      'exportDiagnostics',
      'openLogFolder',
    ] as const
  ).map((type) => z.object({ type: z.literal(type) }).strict()),
])
export type Action = z.infer<typeof actionSchema>

export function unreachableAction(action: never): never {
  throw new Error('不支持的应用操作')
}
