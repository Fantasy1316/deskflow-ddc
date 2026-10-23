import { afterEach, describe, expect, it, vi } from 'vitest'
import { mkdtemp, mkdir, realpath, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { configureApplicationIdentity } from '../electron/core/app-identity'

const roots: string[] = []
afterEach(async () => {
  await Promise.all(roots.splice(0).map((root) => rm(root, { recursive: true, force: true })))
})
async function setup(names: string[]) {
  const root = await mkdtemp(join(tmpdir(), 'ddc-identity-'))
  roots.push(root)
  for (const name of names) {
    const dir = join(root, name, 'screenbridge-v1')
    await mkdir(dir, { recursive: true })
    await writeFile(join(dir, 'settings.json'), '{}')
  }
  return { root, app: { getPath: () => root, setPath: vi.fn(), setName: vi.fn() } }
}
describe('application identity across product renames', () => {
  it.each(['ScreenBridge', 'screenbridge'])(
    'keeps the original macOS keychain namespace for %s data and restores the visible name',
    async (name) => {
      const { root, app } = await setup([name])
      const ready = configureApplicationIdentity(app, 'darwin')
      expect(app.setName.mock.calls).toEqual([['ScreenBridge']])
      expect(app.setPath.mock.calls[0][0]).toBe('userData')
      expect(await realpath(app.setPath.mock.calls[0][1])).toBe(await realpath(join(root, name)))
      ready()
      expect(app.setName.mock.calls).toEqual([['ScreenBridge'], ['Deskflow DDC']])
    },
  )
  it.each([[], ['Deskflow DDC'], ['Deskflow DDC', 'ScreenBridge']])(
    'uses the new namespace for a new installation or existing new-product data: %j',
    async (...names) => {
      const { root, app } = await setup(names)
      configureApplicationIdentity(app, 'darwin')
      expect(app.setName).toHaveBeenCalledWith('Deskflow DDC')
      expect(app.setPath).toHaveBeenCalledWith('userData', join(root, 'Deskflow DDC'))
    },
  )
  it('preserves the Windows product name while reusing the legacy configuration', async () => {
    const { root, app } = await setup(['ScreenBridge'])
    configureApplicationIdentity(app, 'win32')
    expect(app.setName).toHaveBeenCalledWith('Deskflow DDC')
    expect(app.setPath).toHaveBeenCalledWith('userData', join(root, 'ScreenBridge'))
  })
})
