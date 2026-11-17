import { z } from 'zod'

export const latestReleaseApi =
  'https://api.github.com/repos/Fantasy1316/deskflow-ddc/releases/latest'
const requestTimeout = 12_000

export interface UpdateResult {
  status: 'available' | 'current' | 'ahead'
  version: string
  notes: string
  publishedAt: string | null
}

const releaseSchema = z.object({
  tag_name: z.string().max(128),
  draft: z.literal(false),
  prerelease: z.literal(false),
  body: z.string().nullish(),
  published_at: z.string().nullish(),
})

function parseVersion(value: string) {
  const match =
    /^v?(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([\da-zA-Z-]+(?:\.[\da-zA-Z-]+)*))?(?:\+[\da-zA-Z-]+(?:\.[\da-zA-Z-]+)*)?$/.exec(
      value,
    )
  if (!match) throw new Error('无法识别版本号')
  const core = match.slice(1, 4).map(Number)
  const prerelease = match[4]?.split('.') ?? []
  if (
    core.some((part) => !Number.isSafeInteger(part)) ||
    prerelease.some((part) => /^\d+$/.test(part) && part.length > 1 && part.startsWith('0'))
  )
    throw new Error('无法识别版本号')
  return { core, prerelease }
}

/** SemVer precedence; build metadata does not affect whether an update is newer. */
export function compareVersions(left: string, right: string): -1 | 0 | 1 {
  const a = parseVersion(left),
    b = parseVersion(right)
  for (let i = 0; i < 3; i++) {
    if (a.core[i] !== b.core[i]) return a.core[i] > b.core[i] ? 1 : -1
  }
  if (!a.prerelease.length || !b.prerelease.length) {
    return a.prerelease.length === b.prerelease.length ? 0 : a.prerelease.length ? -1 : 1
  }
  for (let i = 0; i < Math.max(a.prerelease.length, b.prerelease.length); i++) {
    const x = a.prerelease[i],
      y = b.prerelease[i]
    if (x === undefined) return -1
    if (y === undefined) return 1
    if (x === y) continue
    const xn = /^\d+$/.test(x),
      yn = /^\d+$/.test(y)
    if (xn !== yn) return xn ? -1 : 1
    if (xn && x.length !== y.length) return x.length > y.length ? 1 : -1
    return x > y ? 1 : -1
  }
  return 0
}

class UpdateCheckError extends Error {}

export async function checkForUpdates(
  currentVersion: string,
  signal?: AbortSignal,
): Promise<UpdateResult> {
  parseVersion(currentVersion)
  if (signal?.aborted) throw new DOMException('已取消检查', 'AbortError')
  const controller = new AbortController()
  const cancel = () => controller.abort()
  signal?.addEventListener('abort', cancel, { once: true })
  let timedOut = false
  const timer = setTimeout(() => {
    timedOut = true
    controller.abort()
  }, requestTimeout)
  try {
    const response = await fetch(latestReleaseApi, {
      headers: { Accept: 'application/vnd.github+json' },
      credentials: 'omit',
      referrerPolicy: 'no-referrer',
      cache: 'no-store',
      redirect: 'error',
      signal: controller.signal,
    })
    if (response.status === 403 || response.status === 429)
      throw new UpdateCheckError('GitHub 暂时限制了查询，请稍后重试，或直接查看版本页面。')
    if (response.status === 404)
      throw new UpdateCheckError('暂未找到公开的正式版本，请前往版本页面查看。')
    if (!response.ok)
      throw new UpdateCheckError(`查询失败（HTTP ${response.status}），请稍后重试。`)
    const payload: unknown = await response.json().catch(() => {
      throw new UpdateCheckError('版本信息格式异常，请前往版本页面查看。')
    })
    if (controller.signal.aborted) throw new DOMException('已取消检查', 'AbortError')
    const parsed = releaseSchema.safeParse(payload)
    if (!parsed.success)
      throw new UpdateCheckError('未获取到有效的正式版本信息，请前往版本页面查看。')
    const release = parsed.data
    let comparison: number
    try {
      if (parseVersion(release.tag_name).prerelease.length) throw new Error('不是正式版本')
      comparison = compareVersions(release.tag_name, currentVersion)
    } catch {
      throw new UpdateCheckError('无法识别最新正式版本，请前往版本页面查看。')
    }
    const date = release.published_at ? Date.parse(release.published_at) : NaN
    return {
      status: comparison > 0 ? 'available' : comparison < 0 ? 'ahead' : 'current',
      version: release.tag_name.replace(/^v/, ''),
      notes: (release.body ?? '').trim().slice(0, 16_000),
      publishedAt: Number.isNaN(date) ? null : new Date(date).toISOString(),
    }
  } catch (error) {
    if (signal?.aborted) throw new DOMException('已取消检查', 'AbortError')
    if (timedOut) throw new UpdateCheckError('检查更新超时，请检查网络后重试。')
    if (error instanceof UpdateCheckError) throw error
    throw new UpdateCheckError('无法连接 GitHub，请检查网络后重试，或直接查看版本页面。')
  } finally {
    clearTimeout(timer)
    signal?.removeEventListener('abort', cancel)
  }
}
