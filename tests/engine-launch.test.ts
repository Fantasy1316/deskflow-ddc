import { describe, expect, it } from 'vitest'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { engineLaunch } from '../electron/core/engine-launch'

describe('isolated Qt launch resources', () => {
  it('reuses Qt key files across Mac restarts while isolating different app profiles', async () => {
    const profile = await mkdtemp(join(tmpdir(), 'sb-launch-test-'))
    const env = { TMPDIR: '/existing/user/temp/', KEEP: 'preserved' }
    const first = await engineLaunch(profile, 'darwin', env),
      other = await engineLaunch(profile + '-other', 'darwin', env)
    try {
      expect(first.directory).not.toBe(other.directory)
      expect(first.env.TMPDIR).toBe(first.directory + '/')
      expect(first.env.KEEP).toBe('preserved')
      expect(env.TMPDIR).toBe('/existing/user/temp/')
      expect(first.ipcPath).toBe(join(first.directory!, 'deskflow-core'))
      expect(Buffer.byteLength(first.ipcPath)).toBeLessThan(104)
      await writeFile(join(first.directory!, 'qt-key'), 'same inode for stale shm recovery')
      const second = await engineLaunch(join(profile, '.'), 'darwin', env)
      expect(second.directory).toBe(first.directory)
      expect(second.ipcPath).toBe(first.ipcPath)
      expect(await readFile(join(second.directory!, 'qt-key'), 'utf8')).toBe(
        'same inode for stale shm recovery',
      )
    } finally {
      await rm(first.directory!, { recursive: true, force: true })
      await rm(other.directory!, { recursive: true, force: true })
      await rm(profile, { recursive: true, force: true })
    }
  })
  it('preserves the Windows runtime environment', async () => {
    const env = { TEMP: 'C:\\Temp', KEEP: 'preserved' }
    const launch = await engineLaunch('/test/profile', 'win32', env)
    expect(launch.directory).toBeUndefined()
    expect(launch.env).toEqual(env)
    expect(launch.env).not.toBe(env)
    expect(launch.ipcPath).toBe('\\\\.\\pipe\\deskflow-core')
  })
})
