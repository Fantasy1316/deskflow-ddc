import { execFileSync } from 'node:child_process'
import {
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import { createHash } from 'node:crypto'
import { resolve, join, dirname } from 'node:path'
import { tmpdir } from 'node:os'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
function argument(name, fallback) {
  const at = process.argv.indexOf(name)
  return at < 0 ? fallback : process.argv[at + 1]
}
const platform = argument('--platform', process.platform)
const arch = argument('--arch', process.arch)
if (!((platform === 'darwin' && arch === 'arm64') || (platform === 'win32' && arch === 'x64'))) {
  throw new Error('Deskflow DDC currently packages Apple Silicon macOS and Windows x64.')
}
const cache = resolve(argument('--cache', join(root, '.engine-cache')))
const destination = resolve(argument('--dest', join(root, 'src-tauri/resources')))
mkdirSync(cache, { recursive: true })
mkdirSync(destination, { recursive: true })
const version = '1.27.0'
const sourceRevision = '081f6478e654a112168879e67a41ce7d7e4ae3af'
const assets = {
  darwin: {
    name: `deskflow-${version}-macos-arm64.dmg`,
    sha256: 'dd21e79e566314394214906d3338c505d337af536fc580c022ee3fadffad0c97',
  },
  win32: {
    name: `deskflow-${version}-win-x64-portable.zip`,
    sha256: '8d19004cc3625623489d96580c1b58fd13ee78185689907fd4478595f93fab9a',
  },
}
const digest = (file) => createHash('sha256').update(readFileSync(file)).digest('hex')
function download(name, url, expected) {
  const target = join(cache, name)
  if (existsSync(target) && digest(target) === expected) return target
  const pending = target + '.download'
  execFileSync(
    'curl',
    [
      '--http1.1',
      '-fL',
      '--retry',
      '2',
      '--connect-timeout',
      '20',
      '--max-time',
      '300',
      url,
      '-o',
      pending,
    ],
    { stdio: 'inherit' },
  )
  if (digest(pending) !== expected) {
    rmSync(pending)
    throw new Error(`Checksum mismatch: ${name}`)
  }
  cpSync(pending, target)
  rmSync(pending)
  return target
}
const asset = assets[platform]
const archive = download(
  asset.name,
  `https://github.com/deskflow/deskflow/releases/download/v${version}/${asset.name}`,
  asset.sha256,
)
const source = download(
  `deskflow-${version}-source.tar.gz`,
  `https://codeload.github.com/deskflow/deskflow/tar.gz/${sourceRevision}`,
  'd2a7b6686deda74095f5a8fe88826d4cdd5554f5e0a5362eec3e654098aa4191',
)
const staging = mkdtempSync(join(tmpdir(), 'ddc-engine-'))
const target = join(destination, 'deskflow')
let mounted = false
try {
  mkdirSync(join(staging, 'content'))
  if (platform === 'darwin') {
    if (process.platform !== 'darwin')
      throw new Error('macOS engine preparation requires macOS to mount the verified DMG')
    execFileSync(
      'hdiutil',
      ['attach', archive, '-readonly', '-nobrowse', '-mountpoint', join(staging, 'content')],
      { input: 'Y\n', stdio: ['pipe', 'pipe', 'pipe'] },
    )
    mounted = true
    const app = readdirSync(join(staging, 'content')).find((name) => name.endsWith('.app'))
    if (!app) throw new Error('Official DMG contains no application')
    rmSync(target, { recursive: true, force: true })
    mkdirSync(target)
    cpSync(join(staging, 'content', app), join(target, 'Deskflow.app'), {
      recursive: true,
      verbatimSymlinks: true,
    })
  } else {
    if (process.platform === 'win32') {
      const script = join(staging, 'extract.ps1')
      writeFileSync(
        script,
        'param([string]$Archive, [string]$Destination)\n$ErrorActionPreference="Stop"\nExpand-Archive -LiteralPath $Archive -DestinationPath $Destination\n',
      )
      execFileSync(
        'powershell.exe',
        [
          '-NoProfile',
          '-NonInteractive',
          '-File',
          script,
          '-Archive',
          archive,
          '-Destination',
          join(staging, 'content'),
        ],
        { stdio: 'inherit', windowsHide: true },
      )
    } else execFileSync('unzip', ['-q', archive, '-d', join(staging, 'content')])
    rmSync(target, { recursive: true, force: true })
    mkdirSync(target)
    cpSync(join(staging, 'content'), join(target, 'runtime'), { recursive: true })
  }
  function findBinary(directory, prefix = '') {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const relative = join(prefix, entry.name)
      if (
        entry.isFile() &&
        entry.name === (platform === 'darwin' ? 'deskflow-core' : 'deskflow-core.exe')
      )
        return relative.replaceAll('\\', '/')
      if (entry.isDirectory()) {
        const found = findBinary(join(directory, entry.name), relative)
        if (found) return found
      }
    }
  }
  const binary = findBinary(target)
  if (!binary) throw new Error('Official distribution contains no deskflow-core')
  const binarySha256 = digest(join(target, binary))
  const runtimeFolder = platform === 'darwin' ? 'Deskflow.app' : 'runtime'
  const runtimeArchive = join(target, 'runtime.tar.gz')
  // One archive preserves Qt framework symlinks through Tauri resource copying.
  execFileSync('tar', ['-czf', runtimeArchive, '-C', target, runtimeFolder], {
    // BSD tar otherwise writes AppleDouble ._* entries. Rust tar extracts
    // those as real files, invalidating the signed Qt framework resource seal.
    env: { ...process.env, COPYFILE_DISABLE: '1' },
  })
  const archiveSha256 = digest(runtimeArchive)
  rmSync(join(target, runtimeFolder), { recursive: true, force: true })
  mkdirSync(join(target, 'licenses'), { recursive: true })
  cpSync(source, join(target, `deskflow-${version}-source.tar.gz`))
  execFileSync('tar', ['-xzf', source, '-C', staging])
  const sourceDir = join(staging, `deskflow-${sourceRevision}`)
  cpSync(join(sourceDir, 'LICENSES'), join(target, 'licenses'), { recursive: true })
  cpSync(join(sourceDir, 'LICENSE'), join(target, 'licenses/DESKFLOW-LICENSE'))
  writeFileSync(
    join(target, 'manifest.json'),
    JSON.stringify(
      {
        version,
        platform,
        arch,
        binary,
        sha256: binarySha256,
        archiveSha256,
        sourceRevision,
        sourceSha256: digest(source),
        upstreamArchiveSha256: asset.sha256,
      },
      null,
      2,
    ) + '\n',
  )
  mkdirSync(join(destination, 'native'), { recursive: true })
  if (platform === 'darwin') {
    execFileSync(process.execPath, [join(root, 'scripts/build-native.mjs')], {
      cwd: root,
      stdio: 'inherit',
    })
    cpSync(join(root, 'resources/bin'), join(destination, 'native/bin'), { recursive: true })
  } else cpSync(join(root, 'native/windows/monitor.ps1'), join(destination, 'native/monitor.ps1'))
  cpSync(join(root, 'resources/vendor/m1ddc/LICENSE'), join(destination, 'native/M1DDC-LICENSE'))
  cpSync(join(root, 'resources/vendor/m1ddc'), join(destination, 'vendor/m1ddc'), {
    recursive: true,
  })
  cpSync(join(root, 'resources/JS-NOTICES.txt'), join(destination, 'JS-NOTICES.txt'))
  cpSync(join(root, 'THIRD_PARTY_NOTICES.md'), join(destination, 'THIRD_PARTY_NOTICES.md'))
  cpSync(join(root, 'LICENSE'), join(destination, 'LICENSE'))
  console.log(`Prepared verified Deskflow ${version} for ${platform}/${arch}: ${target}`)
} finally {
  if (mounted) execFileSync('hdiutil', ['detach', join(staging, 'content')], { stdio: 'inherit' })
  rmSync(staging, { recursive: true, force: true })
}
