import { createHash } from 'node:crypto'
import { mkdir } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { deskflowIpcPath } from './engine-preflight.js'

/** Qt derives both its legacy shared-memory keys and local socket path from TMPDIR. */
export function engineDirectory(dataDir: string, platform = process.platform) {
  const identity = createHash('sha256').update(resolve(dataDir)).digest('hex').slice(0, 16)
  return platform === 'darwin' ? join(tmpdir(), 'sb-core-' + identity) : undefined
}

export async function engineLaunch(
  dataDir: string,
  platform = process.platform,
  env: NodeJS.ProcessEnv = process.env,
) {
  const directory = engineDirectory(dataDir, platform)
  if (directory) await mkdir(directory, { recursive: true, mode: 0o700 })
  // Preserve Qt's key files across stops: its next attach/detach can reclaim stale
  // shared memory after a forced exit. New namespaces would leak one segment per run.
  return {
    env: directory ? { ...env, TMPDIR: directory + '/' } : { ...env },
    ipcPath: deskflowIpcPath(directory, platform),
    directory,
  }
}
