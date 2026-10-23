import { afterEach, describe, expect, it, vi } from 'vitest'
import { mkdtemp, readFile, readdir, rename, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { Store } from '../electron/core/store'
import { safeStorage } from 'electron'

vi.mock('electron', () => ({
  safeStorage: {
    isEncryptionAvailable: () => false,
    decryptString: vi.fn(() => {
      throw new Error('keychain unavailable')
    }),
  },
}))
vi.mock('node:fs/promises', async (importOriginal) => {
  const fs = await importOriginal<typeof import('node:fs/promises')>()
  return { ...fs, rename: vi.fn(fs.rename) }
})
const roots: string[] = []
afterEach(async () => {
  vi.clearAllMocks()
  await Promise.all(roots.splice(0).map((root) => rm(root, { recursive: true, force: true })))
})
async function setup() {
  const root = await mkdtemp(join(tmpdir(), 'sb-store-test-'))
  roots.push(root)
  const store = new Store(root)
  await store.load()
  return store
}
describe('atomic configuration store', () => {
  it('loads an encrypted legacy identity without changing pairing or settings', async () => {
    const store = await setup()
    const file = join(store.dir, 'settings.json')
    const disk = JSON.parse(await readFile(file, 'utf8'))
    disk.secretEncrypted = true
    disk.secret = Buffer.from('ciphertext fixture').toString('base64')
    const text = JSON.stringify(disk)
    await writeFile(file, text)
    vi.mocked(safeStorage.decryptString).mockReturnValueOnce(store.value.pairingCode)
    const loaded = new Store(store.dir)
    await loaded.load()
    expect(loaded.value).toEqual(store.value)
    expect(await readFile(file, 'utf8')).toBe(text)
  })
  it('keeps encrypted settings intact and reports the underlying keychain failure', async () => {
    const store = await setup()
    const file = join(store.dir, 'settings.json')
    const disk = JSON.parse(await readFile(file, 'utf8'))
    disk.secretEncrypted = true
    disk.secret = Buffer.from('ciphertext fixture').toString('base64')
    const text = JSON.stringify(disk)
    await writeFile(file, text)
    const error = await new Store(store.dir).load().catch((error: Error) => error)
    expect(error).toBeInstanceOf(Error)
    expect((error as Error).message).toContain('解密配对密钥')
    expect((error as Error).cause).toMatchObject({ message: 'keychain unavailable' })
    expect(await readFile(file, 'utf8')).toBe(text)
  })
  it('serializes concurrent saves with snapshots, unique temps and last-call ordering', async () => {
    const store = await setup(),
      writes: Promise<void>[] = []
    const saved: string[] = [],
      temps: string[] = []
    const original = await vi.importActual<typeof import('node:fs/promises')>('node:fs/promises')
    vi.mocked(rename).mockImplementation(async (from, to) => {
      temps.push(String(from))
      saved.push(JSON.parse(await readFile(from, 'utf8')).config.deviceName)
      await original.rename(from, to)
    })
    for (let i = 0; i < 40; i++) {
      store.value.config.deviceName = `machine-${i}`
      writes.push(store.save())
    }
    await Promise.all(writes)
    expect(saved).toEqual(Array.from({ length: 40 }, (_, i) => `machine-${i}`))
    expect(new Set(temps).size).toBe(40)
    expect(
      JSON.parse(await readFile(join(store.dir, 'settings.json'), 'utf8')).config.deviceName,
    ).toBe('machine-39')
    expect(await readdir(store.dir)).toEqual(['settings.json'])
  })
  it('preserves the last good file after rename failure and accepts the next save', async () => {
    const store = await setup(),
      before = await readFile(join(store.dir, 'settings.json'), 'utf8')
    vi.mocked(rename).mockRejectedValueOnce(new Error('disk failure'))
    store.value.config.deviceName = 'failed'
    await expect(store.save()).rejects.toThrow('disk failure')
    expect(await readFile(join(store.dir, 'settings.json'), 'utf8')).toBe(before)
    store.value.config.deviceName = 'recovered'
    await store.save()
    expect((await readdir(store.dir)).filter((name) => name.endsWith('.tmp'))).toEqual([])
    const loaded = new Store(store.dir)
    await loaded.load()
    expect(loaded.value.config.deviceName).toBe('recovered')
  })
  it.each(['{broken', '{"id":"bad"}', '{"secretEncrypted":true,"secret":"abc"}'])(
    'does not overwrite corrupt or unreadable identities: %s',
    async (text) => {
      const store = await setup(),
        file = join(store.dir, 'settings.json')
      await writeFile(file, text)
      await expect(new Store(store.dir).load()).rejects.toThrow('未自动覆盖')
      expect(await readFile(file, 'utf8')).toBe(text)
    },
  )
})
