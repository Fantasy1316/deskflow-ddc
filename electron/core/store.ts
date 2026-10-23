import { readFile, mkdir, writeFile, rename, rm } from 'node:fs/promises'
import { join } from 'node:path'
import { randomUUID } from 'node:crypto'
import { hostname } from 'node:os'
import { safeStorage } from 'electron'
import { configSchema, defaults } from './config.js'
import { newPairingCode } from './channel.js'
import type { Config } from '../../src/shared/types.js'
import { z } from 'zod'
import { decodePairingCode } from './channel.js'
const diskIdentity = z.object({
  id: z.string().uuid(),
  trustedPeer: z
    .object({ id: z.string().uuid(), fingerprint: z.string().regex(/^[a-f0-9]{64}$/) })
    .nullable()
    .default(null),
})
export interface Stored {
  id: string
  config: Config
  pairingCode: string
  trustedPeer: { id: string; fingerprint: string } | null
}
export class Store {
  value!: Stored
  private writes: Promise<void> = Promise.resolve()
  constructor(public dir: string) {}
  async load() {
    await mkdir(this.dir, { recursive: true })
    let stage = '读取配置文件'
    try {
      const disk = JSON.parse(await readFile(join(this.dir, 'settings.json'), 'utf8'))
      stage = '解密配对密钥'
      const pairingCode = disk.secretEncrypted
        ? safeStorage.decryptString(Buffer.from(disk.secret, 'base64'))
        : disk.secret
      stage = '校验设备身份'
      const identity = diskIdentity.parse(disk)
      stage = '校验配对密钥'
      decodePairingCode(pairingCode)
      stage = '校验设备设置'
      this.value = { ...identity, config: configSchema.parse(disk.config), pairingCode }
    } catch (error) {
      if (stage !== '读取配置文件' || (error as NodeJS.ErrnoException).code !== 'ENOENT') {
        // Zod errors can contain rejected input. Record only paths and issue codes.
        const cause =
          error instanceof z.ZodError
            ? new Error(
                error.issues.map((issue) => `${issue.path.join('.')}: ${issue.code}`).join('; '),
              )
            : error
        throw new Error(
          `无法读取已保存的配置（${stage}）；为避免丢失配对信息，未自动覆盖。` +
            (stage === '解密配对密钥'
              ? '请确认 macOS 钥匙串或 Windows 凭据访问可用，并允许应用访问原有密钥。'
              : ''),
          { cause },
        )
      }
      this.value = {
        id: randomUUID(),
        config: defaults(hostname().slice(0, 48)),
        pairingCode: newPairingCode(),
        trustedPeer: null,
      }
      await this.save()
    }
  }
  async save() {
    const encrypted = safeStorage.isEncryptionAvailable()
    const { pairingCode, ...other } = this.value
    const data = {
      ...other,
      secretEncrypted: encrypted,
      secret: encrypted ? safeStorage.encryptString(pairingCode).toString('base64') : pairingCode,
    }
    // Capture at invocation, before joining the queue: later mutations must not
    // change the contents promised by this save.
    const text = JSON.stringify(data, null, 2)
    const target = join(this.dir, 'settings.json'),
      temp = target + '.' + randomUUID() + '.tmp'
    const write = this.writes.then(async () => {
      try {
        await writeFile(temp, text, { mode: 0o600, flag: 'wx' })
        await rename(temp, target)
      } finally {
        await rm(temp, { force: true })
      }
    })
    this.writes = write.catch(() => {})
    await write
  }
}
