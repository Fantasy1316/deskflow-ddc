import { afterEach, describe, expect, it, vi } from 'vitest'
import { effectScope } from 'vue'
import { checkForUpdates, compareVersions, latestReleaseApi } from '../src/transport/updates'
import { useUpdateCheck } from '../src/composables/useUpdateCheck'
import config from '../src-tauri/tauri.conf.json'

const release = {
  tag_name: 'v0.3.1',
  draft: false,
  prerelease: false,
  body: '修复连接恢复问题。',
  published_at: '2026-11-10T00:00:00Z',
}
const response = (value: unknown = release) => new Response(JSON.stringify(value))
const scopes: ReturnType<typeof effectScope>[] = []

function updateCheck() {
  const scope = effectScope()
  scopes.push(scope)
  return { scope, ...scope.run(() => useUpdateCheck('0.3.0'))! }
}

function abortableFetch() {
  return vi.fn(
    (_url: unknown, options: RequestInit) =>
      new Promise<Response>((_resolve, reject) => {
        options.signal?.addEventListener(
          'abort',
          () => reject(new DOMException('aborted', 'AbortError')),
          { once: true },
        )
      }),
  )
}

afterEach(() => {
  scopes.splice(0).forEach((scope) => scope.stop())
  vi.unstubAllGlobals()
  vi.useRealTimers()
  vi.restoreAllMocks()
})

describe('release version precedence', () => {
  it.each([
    ['v0.3.0', '0.3.0', 0],
    ['0.2.10', '0.2.9', 1],
    ['0.10.0', '0.9.99', 1],
    ['1.0.0', '0.99.99', 1],
    ['0.3.0', '0.3.1', -1],
    ['0.3.0+build.2', '0.3.0+build.1', 0],
    ['0.3.0-beta.2', '0.3.0-beta.10', -1],
    ['0.3.0-beta.10', '0.3.0-rc.1', -1],
    ['0.3.0-rc.1', '0.3.0', -1],
    ['0.3.0', '0.3.0-rc.1', 1],
    ['0.3.0-alpha', '0.3.0-alpha.1', -1],
    ['0.3.0-alpha.1', '0.3.0-alpha.beta', -1],
    ['0.3.0-99999999999999999999', '0.3.0-99999999999999999998', 1],
  ] as const)('compares %s against %s', (a, b, expected) => {
    expect(compareVersions(a, b)).toBe(expected)
    expect(compareVersions(b, a) + expected).toBe(0)
  })

  it.each(['0.3', '0.03.0', '0.3.0-beta.01', 'latest', '0.3.0-', '9007199254740992.0.0'])(
    'rejects an ambiguous or invalid version %s',
    (value) => {
      expect(() => compareVersions(value, '0.3.0')).toThrow()
    },
  )
})

describe('GitHub update checks', () => {
  it('checks only the fixed public endpoint allowed by the desktop CSP, without credentials', async () => {
    const fetcher = vi.fn().mockResolvedValue(response())
    vi.stubGlobal('fetch', fetcher)
    const result = await checkForUpdates('0.3.0')
    expect(result).toEqual({
      status: 'available',
      version: '0.3.1',
      notes: release.body,
      publishedAt: '2026-11-10T00:00:00.000Z',
    })
    expect(fetcher).toHaveBeenCalledExactlyOnceWith(
      latestReleaseApi,
      expect.objectContaining({
        credentials: 'omit',
        referrerPolicy: 'no-referrer',
        cache: 'no-store',
        redirect: 'error',
      }),
    )
    const connect = config.app.security.csp
      .split(';')
      .find((value) => value.trim().startsWith('connect-src'))!
    expect(connect.split(/\s+/)).toContain(latestReleaseApi)
  })

  it.each([
    ['0.3.1', 'current'],
    ['0.3.2', 'ahead'],
    ['0.3.1-beta.1', 'available'],
  ])(
    'compares the installed version %s with the latest stable release',
    async (version, status) => {
      vi.stubGlobal('fetch', vi.fn().mockResolvedValue(response()))
      expect((await checkForUpdates(version)).status).toBe(status)
    },
  )

  it.each([
    [403, 'GitHub 暂时限制'],
    [429, 'GitHub 暂时限制'],
    [404, '暂未找到'],
    [503, 'HTTP 503'],
  ])('does not misreport HTTP %s as being up to date', async (status, message) => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response('', { status })))
    await expect(checkForUpdates('0.3.0')).rejects.toThrow(message)
  })

  it.each([
    { ...release, draft: true },
    { ...release, prerelease: true },
    { tag_name: 'v0.3.1' },
    { ...release, tag_name: 'latest' },
    { ...release, tag_name: 'v0.3.1-beta.1' },
  ])('rejects unpublished, preview or malformed release metadata', async (value) => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(response(value)))
    await expect(checkForUpdates('0.3.0')).rejects.toThrow()
  })

  it('handles malformed JSON and unavailable networks', async () => {
    const fetcher = vi
      .fn()
      .mockResolvedValueOnce(new Response('<html>gateway error</html>'))
      .mockRejectedValueOnce(new TypeError('Failed to fetch'))
    vi.stubGlobal('fetch', fetcher)
    await expect(checkForUpdates('0.3.0')).rejects.toThrow('格式异常')
    await expect(checkForUpdates('0.3.0')).rejects.toThrow('无法连接 GitHub')
  })

  it('aborts a stalled request after 12 seconds and releases its timer', async () => {
    vi.useFakeTimers()
    const fetcher = abortableFetch()
    vi.stubGlobal('fetch', fetcher)
    const result = expect(checkForUpdates('0.3.0')).rejects.toThrow('超时')
    await vi.advanceTimersByTimeAsync(12_000)
    await result
    expect(fetcher.mock.calls[0][1].signal?.aborted).toBe(true)
    expect(vi.getTimerCount()).toBe(0)
  })

  it('cancels an in-flight request and never starts an already canceled one', async () => {
    const fetcher = abortableFetch(),
      controller = new AbortController()
    vi.stubGlobal('fetch', fetcher)
    const result = expect(checkForUpdates('0.3.0', controller.signal)).rejects.toMatchObject({
      name: 'AbortError',
    })
    controller.abort()
    await result
    await expect(checkForUpdates('0.3.0', controller.signal)).rejects.toMatchObject({
      name: 'AbortError',
    })
    expect(fetcher).toHaveBeenCalledOnce()
  })
})

describe('update check lifecycle', () => {
  it('waits for a user request and coalesces repeated clicks while checking', async () => {
    let resolve!: (value: Response) => void
    const fetcher = vi.fn().mockReturnValue(
      new Promise<Response>((done) => {
        resolve = done
      }),
    )
    vi.stubGlobal('fetch', fetcher)
    const { state, check } = updateCheck()
    expect(state.value.status).toBe('idle')
    expect(fetcher).not.toHaveBeenCalled()
    const first = check()
    await check()
    expect(state.value.status).toBe('checking')
    expect(fetcher).toHaveBeenCalledOnce()
    resolve(response())
    await first
    expect(state.value).toMatchObject({ status: 'checked', result: { status: 'available' } })
  })

  it('lets the user retry a failed check', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockRejectedValueOnce(new Error('offline')).mockResolvedValueOnce(response()),
    )
    const { state, check } = updateCheck()
    await check()
    expect(state.value).toMatchObject({
      status: 'error',
      message: expect.stringContaining('无法连接'),
    })
    await check()
    expect(state.value.status).toBe('checked')
  })

  it('cancels on page disposal and ignores late results', async () => {
    let resolve!: (value: Response) => void
    const fetcher = vi.fn().mockReturnValue(
      new Promise<Response>((done) => {
        resolve = done
      }),
    )
    vi.stubGlobal('fetch', fetcher)
    const { state, check, scope } = updateCheck()
    const pending = check()
    scope.stop()
    expect(fetcher.mock.calls[0][1].signal.aborted).toBe(true)
    resolve(response())
    await pending
    expect(state.value.status).toBe('checking')
    await check()
    expect(fetcher).toHaveBeenCalledOnce()
  })
})
