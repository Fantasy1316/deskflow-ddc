import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { existsSync, readFileSync, renameSync, rmSync } from 'node:fs'
import { join } from 'node:path'

const digest = (path) => createHash('sha256').update(readFileSync(path)).digest('hex')

export function downloadVerified({ cache, name, url, sha256 }, run = execFileSync) {
  const target = join(cache, name)
  if (existsSync(target) && digest(target) === sha256) return target
  const pending = target + '.download'
  const fetch = (address) =>
    run(
      'curl',
      [
        '--http1.1',
        '-fL',
        '--retry',
        '3',
        '--retry-delay',
        '3',
        '--connect-timeout',
        '20',
        '--max-time',
        '300',
        address,
        '-o',
        pending,
      ],
      { stdio: 'inherit' },
    )
  try {
    try {
      fetch(url)
    } catch (error) {
      const fresh = new URL(url)
      if (
        fresh.origin !== 'https://github.com' ||
        !/^\/[^/]+\/[^/]+\/releases\/download\//.test(fresh.pathname)
      )
        throw error
      // A cached release redirect/error can survive curl retries of the same URL.
      // Request the same official asset again; its pinned digest remains mandatory.
      fresh.searchParams.set('deskflow-ddc-retry', `${Date.now()}-${process.pid}`)
      console.warn(`Retrying official release download with a fresh request: ${name}`)
      rmSync(pending, { force: true })
      fetch(fresh.toString())
    }
    if (digest(pending) !== sha256) throw new Error(`Checksum mismatch: ${name}`)
    renameSync(pending, target)
    return target
  } finally {
    rmSync(pending, { force: true })
  }
}
