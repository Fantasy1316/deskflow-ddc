import { afterEach, describe, expect, it, vi } from 'vitest'
import { invoke, isTauri } from '@tauri-apps/api/core'
import capability from '../src-tauri/capabilities/main.json'
import { followProjectLink, projectLinks, type ProjectLink } from '../src/transport/project-links'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), isTauri: vi.fn() }))
afterEach(() => vi.resetAllMocks())

describe('project links', () => {
  it('keeps native opening limited to the project URLs allowed by the main window', async () => {
    vi.mocked(isTauri).mockReturnValue(true)
    const permission = capability.permissions.find(
      (item) => typeof item === 'object' && item.identifier === 'opener:allow-open-url',
    )
    if (!permission || typeof permission === 'string') throw new Error('Missing opener permission')
    expect(permission.allow.map(({ url }) => url).sort()).toEqual(
      Object.values(projectLinks).sort(),
    )
    expect(capability.windows).toEqual(['main'])
    for (const link of Object.keys(projectLinks) as ProjectLink[]) {
      const preventDefault = vi.fn()
      await followProjectLink({ preventDefault }, link)
      expect(preventDefault).toHaveBeenCalledOnce()
      expect(invoke).toHaveBeenLastCalledWith('plugin:opener|open_url', {
        url: projectLinks[link],
      })
    }
  })

  it('leaves browser preview links to normal anchor navigation', async () => {
    vi.mocked(isTauri).mockReturnValue(false)
    const preventDefault = vi.fn()
    await followProjectLink({ preventDefault }, 'repository')
    expect(preventDefault).not.toHaveBeenCalled()
    expect(invoke).not.toHaveBeenCalled()
  })

  it('rejects unknown links before invoking the desktop opener', async () => {
    vi.mocked(isTauri).mockReturnValue(true)
    for (const link of ['https://example.com', '__proto__', 'constructor']) {
      await expect(
        followProjectLink({ preventDefault: vi.fn() }, link as ProjectLink),
      ).rejects.toThrow('未知的项目链接')
    }
    expect(invoke).not.toHaveBeenCalled()
  })

  it('propagates a browser launch failure for a visible, copyable fallback', async () => {
    vi.mocked(isTauri).mockReturnValue(true)
    vi.mocked(invoke).mockRejectedValue(new Error('browser unavailable'))
    await expect(followProjectLink({ preventDefault: vi.fn() }, 'feedback')).rejects.toThrow(
      'browser unavailable',
    )
  })
})
