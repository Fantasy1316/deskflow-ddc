// Offline development packaging. Pass a verified Electron release ZIP; no downloads or system installs.
import { createRequire } from 'node:module'
import { readFile, writeFile, mkdir, cp, rename, rm, realpath } from 'node:fs/promises'
import { createHash } from 'node:crypto'
import { execFileSync } from 'node:child_process'
import { resolve, join, dirname, basename } from 'node:path'
const require = createRequire(import.meta.url)
const [platform, archiveArg] = process.argv.slice(2)
if (!['darwin-arm64', 'win32-x64'].includes(platform) || !archiveArg)
  throw new Error(
    '用法：node scripts/package-local.mjs darwin-arm64|win32-x64 /path/to/electron-vVERSION-PLATFORM.zip',
  )
const archive = resolve(archiveArg),
  metadata = JSON.parse(await readFile('package.json', 'utf8'))
const electron = JSON.parse(await readFile('node_modules/electron/package.json', 'utf8'))
const checksums = JSON.parse(await readFile('node_modules/electron/checksums.json', 'utf8'))
const archiveName = `electron-v${electron.version}-${platform}.zip`
const hash = createHash('sha256')
  .update(await readFile(archive))
  .digest('hex')
if (!checksums[archiveName] || hash !== checksums[archiveName])
  throw new Error('Electron ZIP SHA-256 校验失败，未执行解压')
const output = resolve('release', metadata.version, platform),
  mac = platform.startsWith('darwin')
await mkdir(output, { recursive: true })
execFileSync('unzip', ['-oq', archive, '-d', output], { stdio: 'inherit' })
const bundle = join(output, 'Deskflow DDC.app')
if (mac) {
  await rm(bundle, { recursive: true, force: true })
  await rename(join(output, 'Electron.app'), bundle)
} else await rename(join(output, 'electron.exe'), join(output, 'Deskflow DDC.exe'))
const resources = mac ? join(bundle, 'Contents/Resources') : join(output, 'resources')
await rm(join(resources, 'default_app.asar'), { force: true })
const app = join(resources, 'app')
await mkdir(app, { recursive: true })
for (const dir of ['dist', 'dist-electron']) await cp(dir, join(app, dir), { recursive: true })
await writeFile(
  join(app, 'package.json'),
  JSON.stringify(
    {
      name: metadata.name,
      productName: 'Deskflow DDC',
      license: metadata.license,
      version: metadata.version,
      type: 'module',
      main: metadata.main,
      dependencies: {
        selfsigned: metadata.dependencies.selfsigned,
        zod: metadata.dependencies.zod,
      },
    },
    null,
    2,
  ),
)
const copied = new Map()
async function dependency(name, fromRequire) {
  let root = dirname(fromRequire.resolve(name))
  for (;;) {
    try {
      const p = JSON.parse(await readFile(join(root, 'package.json'), 'utf8'))
      if (p.name === name) break
    } catch {}
    const parent = dirname(root)
    if (parent === root) throw new Error('找不到依赖 ' + name)
    root = parent
  }
  root = await realpath(root)
  const pkg = JSON.parse(await readFile(join(root, 'package.json'), 'utf8'))
  if (copied.has(name)) {
    if (copied.get(name) !== pkg.version) throw new Error('依赖版本冲突 ' + name)
    return
  }
  copied.set(name, pkg.version)
  await cp(root, join(app, 'node_modules', name), {
    recursive: true,
    dereference: true,
    filter: (path) => basename(path) !== 'node_modules',
  })
  const childRequire = createRequire(join(root, 'package.json'))
  for (const dep of Object.keys(pkg.dependencies ?? {})) await dependency(dep, childRequire)
}
for (const name of ['selfsigned', 'zod']) await dependency(name, require)
const native = join(resources, 'native')
await mkdir(native, { recursive: true })
for (const file of ['m1ddc-LICENSE']) await cp(join('resources', file), join(native, file))
await cp('resources/icons', join(native, 'icons'), { recursive: true })
await cp('LICENSE', join(resources, 'LICENSE'))
await cp('THIRD_PARTY_NOTICES.md', join(resources, 'THIRD_PARTY_NOTICES.md'))
await cp('resources/JS-NOTICES.txt', join(resources, 'JS-NOTICES.txt'))
if (mac) {
  await cp(join(output, 'LICENSE'), join(resources, 'Electron-LICENSE'))
  await cp(join(output, 'LICENSES.chromium.html'), join(resources, 'LICENSES.chromium.html'))
  await rename(join(bundle, 'Contents/MacOS/Electron'), join(bundle, 'Contents/MacOS/Deskflow DDC'))
  await cp('resources/bin', join(native, 'bin'), { recursive: true })
  await cp('resources/icons/app.icns', join(resources, 'Deskflow DDC.icns'))
  const plist = join(bundle, 'Contents/Info.plist')
  const python =
    'import plistlib,sys\np=sys.argv[1]\nwith open(p,"rb") as f:d=plistlib.load(f)\nd.update(CFBundleIconFile="Deskflow DDC.icns",CFBundleExecutable="Deskflow DDC",CFBundleIdentifier="app.deskflow-ddc.desktop",CFBundleName="Deskflow DDC",CFBundleDisplayName="Deskflow DDC",CFBundleShortVersionString=sys.argv[2],CFBundleVersion=sys.argv[2],NSLocalNetworkUsageDescription="连接您配对的另一台电脑，协调显示器和键鼠切换。")\nwith open(p,"wb") as f:plistlib.dump(d,f)'
  execFileSync('python3', ['-c', python, plist, metadata.version])
  execFileSync('codesign', ['--force', '--deep', '--sign', '-', bundle], { stdio: 'inherit' })
} else {
  await cp('native/windows/monitor.ps1', join(native, 'monitor.ps1'))
  const { patchWindowsExecutable } = await import('./windows-resources.mjs')
  await patchWindowsExecutable(
    join(output, 'Deskflow DDC.exe'),
    'resources/icons/app.ico',
    metadata.version,
  )
}
await writeFile(
  join(output, 'BUILD-INFO.json'),
  JSON.stringify(
    {
      appVersion: metadata.version,
      electronVersion: electron.version,
      platform,
      archiveSha256: hash,
      packaging: 'local development directory; unsigned / ad-hoc signed',
      dependencies: Object.fromEntries(copied),
      iconSha256: createHash('sha256')
        .update(await readFile('resources/icons/app.' + (mac ? 'icns' : 'ico')))
        .digest('hex'),
    },
    null,
    2,
  ),
)
console.log('已校验并打包：' + output)
