import { createRequire } from 'node:module'
import { readFile, writeFile, rename } from 'node:fs/promises'
const require = createRequire(import.meta.url)
// Use the same resource editor bundled with our existing electron-builder dependency.
const builderRequire = createRequire(require.resolve('electron-builder'))
const libRequire = createRequire(builderRequire.resolve('app-builder-lib'))
const { NtExecutable, NtExecutableResource, Resource, Data } = libRequire('resedit')

export async function patchWindowsExecutable(path, iconPath, version) {
  const original = await readFile(path)
  // Resource changes invalidate Electron's vendor signature; output is an unsigned development build.
  const exe = NtExecutable.from(original, { ignoreCert: true })
  const resources = NtExecutableResource.from(exe)
  const before = resources.entries
    .filter((entry) => entry.type !== 3 && entry.type !== 14 && entry.type !== 16)
    .map((entry) => ({
      type: entry.type,
      id: entry.id,
      lang: entry.lang,
      data: Buffer.from(entry.bin),
    }))
  const icon = Data.IconFile.from(await readFile(iconPath))
  const groups = Resource.IconGroupEntry.fromEntries(resources.entries)
  for (const group of groups.length ? groups : [{ id: 1, lang: 1033 }]) {
    Resource.IconGroupEntry.replaceIconsForResource(
      resources.entries,
      group.id,
      group.lang,
      icon.icons.map((item) => item.data),
    )
  }
  const versions = Resource.VersionInfo.fromEntries(resources.entries)
  for (const info of versions.length ? versions : [Resource.VersionInfo.createEmpty()]) {
    info.setFileVersion(version)
    info.setProductVersion(version)
    const languages = info.getAllLanguagesForStringValues()
    for (const language of languages.length ? languages : [{ lang: 1033, codepage: 1200 }]) {
      info.setStringValues(language, {
        FileDescription: 'Deskflow DDC',
        ProductName: 'Deskflow DDC',
        InternalName: 'Deskflow DDC',
        OriginalFilename: 'Deskflow DDC.exe',
        FileVersion: version,
        ProductVersion: version,
      })
    }
    info.outputToResourceEntries(resources.entries)
  }
  resources.outputResource(exe)
  const output = Buffer.from(exe.generate())
  const verified = NtExecutableResource.from(NtExecutable.from(output))
  for (const entry of before) {
    const actual = verified.entries.find(
      (item) => item.type === entry.type && item.id === entry.id && item.lang === entry.lang,
    )
    if (!actual || !Buffer.from(actual.bin).equals(entry.data))
      throw new Error('Windows resource preservation check failed')
  }
  const verifiedGroups = Resource.IconGroupEntry.fromEntries(verified.entries)
  if (!verifiedGroups.length) throw new Error('Missing embedded application icon')
  for (const group of verifiedGroups) {
    const images = group.getIconItemsFromEntries(verified.entries)
    if (
      images.length !== icon.icons.length ||
      images.some(
        (image, index) =>
          !Buffer.from(image.bin ?? image.generate()).equals(
            Buffer.from(icon.icons[index].data.bin ?? icon.icons[index].data.generate()),
          ),
      )
    ) {
      throw new Error('Embedded Windows application icon differs from supplied ICO')
    }
  }
  const temp = path + '.new'
  await writeFile(temp, output)
  await rename(temp, path)
  console.log('Windows EXE application icon and version verified:', version)
}
