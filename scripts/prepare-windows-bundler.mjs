// Matches the official NSIS toolchain used by the locked Tauri CLI 2.12.4.
import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { cpSync, mkdirSync, mkdtempSync, readFileSync, renameSync, rmSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { downloadVerified } from './download-verified.mjs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const cli = JSON.parse(readFileSync(join(root, 'node_modules/@tauri-apps/cli/package.json')))
if (cli.version !== '2.12.4')
  throw new Error(`Update the pinned NSIS toolchain for Tauri CLI ${cli.version}`)
const destinationArgument = process.argv.indexOf('--dest')
if (destinationArgument < 0 && process.platform !== 'win32')
  throw new Error('Windows bundler preparation requires Windows or an explicit --dest')
const destination = resolve(
  destinationArgument < 0
    ? join(process.env.LOCALAPPDATA, 'tauri')
    : process.argv[destinationArgument + 1],
)
const cache = join(root, '.engine-cache')
mkdirSync(cache, { recursive: true })
mkdirSync(destination, { recursive: true })
const archive = downloadVerified({
  cache,
  name: 'nsis-3.13.zip',
  url: 'https://github.com/tauri-apps/binary-releases/releases/download/nsis-3.13/nsis-3.13.zip',
  sha256: 'ba63dffc4410ee89193e1cb5a41989991bd77c61068da17e3156d136b7b0b3d8',
})
const plugin = downloadVerified({
  cache,
  name: 'nsis_tauri_utils-v0.5.3.dll',
  url: 'https://github.com/tauri-apps/nsis-tauri-utils/releases/download/nsis_tauri_utils-v0.5.3/nsis_tauri_utils.dll',
  sha256: '5ba143b5db4a87d32d6e7802e033330aae56cbceabe0d1e3ba41948385ad4709',
})
const staging = mkdtempSync(join(destination, 'nsis-verified-'))
try {
  execFileSync('tar', ['-xf', archive, '-C', staging], { stdio: 'inherit' })
  const toolset = join(staging, 'nsis-3.13')
  const executableHash = createHash('sha1')
    .update(readFileSync(join(toolset, 'NSIS.exe')))
    .digest('hex')
  if (executableHash !== '6d45d4e8ccf18fe1fdac9164058f356102a2ecf2')
    throw new Error('NSIS executable does not match the locked Tauri CLI')
  const plugins = join(toolset, 'Plugins/x86-unicode/additional')
  mkdirSync(plugins, { recursive: true })
  cpSync(plugin, join(plugins, 'nsis_tauri_utils.dll'))
  const target = join(destination, 'NSIS')
  rmSync(target, { recursive: true, force: true })
  renameSync(toolset, target)
  console.log(`Prepared verified official NSIS toolchain: ${target}`)
} finally {
  rmSync(staging, { recursive: true, force: true })
}
