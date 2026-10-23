import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { deskflowIpcPath } from './engine-preflight.js'

/** Qt derives both its legacy shared-memory keys and local socket path from TMPDIR. */
export async function engineLaunch(
  platform = process.platform,
  env: NodeJS.ProcessEnv = process.env,
) {
  // A new directory per launch also isolates retries after an abnormal engine exit.
  const directory = platform === 'darwin' ? await mkdtemp(join(tmpdir(), 'sb-core-')) : undefined
  return {
    env: directory ? { ...env, TMPDIR: directory + '/' } : { ...env },
    ipcPath: deskflowIpcPath(directory, platform),
    directory,
    cleanup: async () => {
      if (directory) await rm(directory, { recursive: true, force: true })
    },
  }
}
