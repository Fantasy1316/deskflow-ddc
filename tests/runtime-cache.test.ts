import { afterEach, describe, expect, it } from 'vitest'
import { mkdtemp, mkdir, writeFile, readFile, stat, rm, readdir } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { prepareRuntimeCopy } from '../electron/core/runtime-cache'
const roots: string[] = []
afterEach(async () => {
  await Promise.all(roots.splice(0).map((root) => rm(root, { recursive: true, force: true })))
})
describe('private Deskflow runtime cache', () => {
  it('skips unchanged runtimes, rebuilds updates without stale DLLs, preserves private settings', async () => {
    const root = await mkdtemp(join(tmpdir(), 'sb-runtime-copy-'))
    roots.push(root)
    const source = join(root, 'installed'),
      target = join(root, 'private')
    await mkdir(join(source, 'settings'), { recursive: true })
    await writeFile(join(source, 'deskflow-core.exe'), 'version-1')
    await writeFile(join(source, 'old.dll'), 'old')
    await writeFile(join(source, 'settings', 'secret'), 'vendor settings')
    await prepareRuntimeCopy(source, target)
    expect(await readdir(target)).not.toContain('settings')
    const before = await stat(join(target, 'deskflow-core.exe'))
    await prepareRuntimeCopy(source, target)
    expect((await stat(join(target, 'deskflow-core.exe'))).ino).toBe(before.ino)
    await mkdir(join(target, 'settings'))
    await writeFile(join(target, 'settings', 'identity'), 'private')
    await writeFile(join(source, 'deskflow-core.exe'), 'version-2-longer')
    await rm(join(source, 'old.dll'))
    await prepareRuntimeCopy(source, target)
    expect(await readFile(join(target, 'deskflow-core.exe'), 'utf8')).toBe('version-2-longer')
    expect(await readdir(target)).not.toContain('old.dll')
    expect(await readFile(join(target, 'settings', 'identity'), 'utf8')).toBe('private')
    expect((await readdir(root)).sort()).toEqual(['installed', 'private'])
  })
  it('repairs an incomplete private copy without trusting the manifest alone', async () => {
    const root = await mkdtemp(join(tmpdir(), 'sb-runtime-repair-'))
    roots.push(root)
    const source = join(root, 'installed'),
      target = join(root, 'private')
    await mkdir(source)
    await writeFile(join(source, 'deskflow-core.exe'), 'version')
    await prepareRuntimeCopy(source, target)
    await rm(join(target, 'deskflow-core.exe'))
    await prepareRuntimeCopy(source, target)
    expect(await readFile(join(target, 'deskflow-core.exe'), 'utf8')).toBe('version')
  })
})
