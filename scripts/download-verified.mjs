import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { closeSync, existsSync, openSync, readFileSync, renameSync, rmSync } from 'node:fs'
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
      const release = fresh.pathname.match(
        /^\/([^/]+)\/([^/]+)\/releases\/download\/([^/]+)\/(.+)$/,
      )
      if (fresh.origin !== 'https://github.com' || !release) throw error
      // A cached release redirect/error can survive curl retries of the same URL.
      // Request the same official asset again; its pinned digest remains mandatory.
      fresh.searchParams.set('deskflow-ddc-retry', `${Date.now()}-${process.pid}`)
      console.warn(`Retrying official release download with a fresh request: ${name}`)
      rmSync(pending, { force: true })
      try {
        fetch(fresh.toString())
      } catch {
        // gh handles authentication without placing tokens in child-process args/errors.
        // The API uses the same official asset, through a different redirect endpoint.
        const [, owner, repo, tag, encodedName] = release
        const assetName = decodeURIComponent(encodedName)
        const metadata = JSON.parse(
          run('gh', ['api', `repos/${owner}/${repo}/releases/tags/${tag}`], {
            stdio: ['ignore', 'pipe', 'inherit'],
          }).toString(),
        )
        const asset = metadata.assets?.find((item) => item.name === assetName)
        if (!Number.isSafeInteger(asset?.id) || asset.id <= 0)
          throw new Error(`Official release contains no asset: ${assetName}`)
        console.warn(`Retrying through the official GitHub asset API: ${name}`)
        const output = openSync(pending, 'w')
        try {
          run(
            'gh',
            [
              'api',
              `repos/${owner}/${repo}/releases/assets/${asset.id}`,
              '-H',
              'Accept: application/octet-stream',
            ],
            { stdio: ['ignore', output, 'inherit'] },
          )
        } finally {
          closeSync(output)
        }
      }
    }
    if (digest(pending) !== sha256) throw new Error(`Checksum mismatch: ${name}`)
    renameSync(pending, target)
    return target
  } finally {
    rmSync(pending, { force: true })
  }
}
