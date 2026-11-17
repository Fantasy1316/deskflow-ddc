import { invoke, isTauri } from '@tauri-apps/api/core'

export const projectLinks = {
  repository: 'https://github.com/Fantasy1316/deskflow-ddc',
  guide: 'https://github.com/Fantasy1316/deskflow-ddc/blob/main/README.md',
  releases: 'https://github.com/Fantasy1316/deskflow-ddc/releases',
  feedback: 'https://github.com/Fantasy1316/deskflow-ddc/issues',
  license: 'https://github.com/Fantasy1316/deskflow-ddc/blob/main/LICENSE',
  notices: 'https://github.com/Fantasy1316/deskflow-ddc/blob/main/THIRD_PARTY_NOTICES.md',
} as const

export type ProjectLink = keyof typeof projectLinks

/** Keep ordinary links in web preview; use the system browser in the desktop app. */
export async function followProjectLink(
  event: Pick<MouseEvent, 'preventDefault'>,
  link: ProjectLink,
): Promise<void> {
  if (!isTauri()) return
  event.preventDefault()
  if (!Object.hasOwn(projectLinks, link)) throw new Error('未知的项目链接')
  await invoke<void>('plugin:opener|open_url', { url: projectLinks[link] })
}
