import { describe, expect, it } from 'vitest'
import { access, readFile, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { engineLaunch } from '../electron/core/engine-launch'

describe('isolated Qt launch resources', () => {
  it('gives each Mac launch a separate temp namespace and matching IPC path', async () => {
    const env = { TMPDIR: '/existing/user/temp/', KEEP: 'preserved' }
    const first = await engineLaunch('darwin', env),
      second = await engineLaunch('darwin', env)
    try {
      expect(first.directory).not.toBe(second.directory)
      expect(first.env.TMPDIR).toBe(first.directory + '/')
      expect(first.env.KEEP).toBe('preserved')
      expect(env.TMPDIR).toBe('/existing/user/temp/')
      expect(first.ipcPath).toBe(join(first.directory!, 'deskflow-core'))
      expect(Buffer.byteLength(first.ipcPath)).toBeLessThan(104)
      await writeFile(join(second.directory!, 'keep'), 'other engine')
      await first.cleanup()
      await expect(access(first.directory!)).rejects.toThrow()
      expect(await readFile(join(second.directory!, 'keep'), 'utf8')).toBe('other engine')
    } finally {
      await first.cleanup()
      await second.cleanup()
    }
  })
  it('preserves the Windows runtime environment', async () => {
    const env = { TEMP: 'C:\\Temp', KEEP: 'preserved' }
    const launch = await engineLaunch('win32', env)
    expect(launch.directory).toBeUndefined()
    expect(launch.env).toEqual(env)
    expect(launch.env).not.toBe(env)
    expect(launch.ipcPath).toBe('\\\\.\\pipe\\deskflow-core')
    await launch.cleanup()
  })
})
