import { execFileSync } from 'node:child_process'
import { mkdirSync } from 'node:fs'
import { resolve } from 'node:path'
if (process.platform !== 'darwin') {
  console.log('Windows 使用随附的原生 DXVA2 PowerShell 桥，无需编译。')
  process.exit(0)
}
if (process.arch !== 'arm64') throw new Error('此版本的原生 DDC 后端需要 Apple Silicon Mac。')
mkdirSync('resources/bin', { recursive: true })
mkdirSync('.native-cache', { recursive: true })
const cache = resolve('.native-cache')
execFileSync(
  'clang',
  [
    '-fmodules',
    '-mmacosx-version-min=14.0',
    `-fmodules-cache-path=${cache}`,
    '-Iresources/vendor/m1ddc/headers',
    '-framework',
    'CoreDisplay',
    '-framework',
    'Foundation',
    '-framework',
    'IOKit',
    '-framework',
    'CoreGraphics',
    'resources/vendor/m1ddc/sources/i2c.m',
    'resources/vendor/m1ddc/sources/ioregistry.m',
    'resources/vendor/m1ddc/sources/m1ddc.m',
    '-o',
    'resources/bin/m1ddc',
  ],
  { stdio: 'inherit' },
)
execFileSync(
  'swiftc',
  [
    '-target',
    'arm64-apple-macosx14.0',
    '-module-cache-path',
    cache,
    'native/mac-hotkey.swift',
    '-o',
    'resources/bin/mac-hotkey',
  ],
  { stdio: 'inherit' },
)
console.log('原生 DDC 与快捷键桥已编译。')
