import {
  cp,
  mkdir,
  readFile,
  readdir,
  realpath,
  rename,
  rm,
  stat,
  writeFile,
} from 'node:fs/promises'
import { join, relative, resolve, sep } from 'node:path'
import { randomUUID } from 'node:crypto'

async function inventory(
  root: string,
  prefix = '',
): Promise<Record<string, { size: number; mtime: number }>> {
  const result: Record<string, { size: number; mtime: number }> = {}
  for (const entry of (await readdir(join(root, prefix), { withFileTypes: true })).sort((a, b) =>
    a.name.localeCompare(b.name),
  )) {
    if (entry.name === 'settings' || (!prefix && entry.name === '.screenbridge-runtime.json'))
      continue
    const name = join(prefix, entry.name)
    if (entry.isSymbolicLink()) throw new Error('Deskflow 运行目录包含符号链接，无法安全复制')
    if (entry.isDirectory()) Object.assign(result, await inventory(root, name))
    else if (entry.isFile()) {
      const info = await stat(join(root, name))
      result[name] = { size: info.size, mtime: info.mtimeMs }
    }
  }
  return result
}

/** Rebuild only when source or cached file metadata changes; stage before swapping. */
export async function prepareRuntimeCopy(source: string, target: string) {
  source = await realpath(source)
  target = resolve(target)
  if (source === target) return
  if (target.startsWith(source + sep) || source.startsWith(target + sep))
    throw new Error('Deskflow 源目录和运行目录不能相互包含')
  const files = await inventory(source)
  if (!files['deskflow-core.exe']) throw new Error('所选目录缺少 deskflow-core.exe')
  try {
    const saved = JSON.parse(await readFile(join(target, '.screenbridge-runtime.json'), 'utf8'))
    if (
      saved.source === source &&
      JSON.stringify(saved.files) === JSON.stringify(files) &&
      JSON.stringify(saved.copied) === JSON.stringify(await inventory(target))
    )
      return
  } catch {
    /* A missing/incomplete cache must be rebuilt. */
  }
  const stage = target + '.stage-' + randomUUID(),
    backup = target + '.old-' + randomUUID()
  let movedOld = false,
    installed = false
  try {
    await mkdir(stage, { recursive: true })
    await cp(source, stage, {
      recursive: true,
      filter: (path) => !relative(source, path).split(sep).includes('settings'),
    })
    // Keep our private generated settings across an engine upgrade.
    try {
      await cp(join(target, 'settings'), join(stage, 'settings'), { recursive: true })
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error
    }
    await writeFile(
      join(stage, '.screenbridge-runtime.json'),
      JSON.stringify({ source, files, copied: await inventory(stage) }),
    )
    try {
      await rename(target, backup)
      movedOld = true
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error
    }
    try {
      await rename(stage, target)
      installed = true
    } catch (error) {
      if (movedOld) await rename(backup, target)
      throw error
    }
  } finally {
    await rm(stage, { recursive: true, force: true })
    if (installed && movedOld) await rm(backup, { recursive: true, force: true })
  }
}
