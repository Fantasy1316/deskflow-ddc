import { execFileSync } from 'node:child_process'
import { mkdirSync } from 'node:fs'
import { resolve } from 'node:path'
import { macArchitecture, verifyMacArchitecture } from './macos-architecture.mjs'
if (process.platform === 'linux') {
  mkdirSync('resources/bin', { recursive: true })
  execFileSync(
    'cc',
    [
      '-std=c11',
      '-O2',
      '-Wall',
      '-Wextra',
      '-Werror',
      'native/linux/input.c',
      '-o',
      'resources/bin/linux-input',
      '-lX11',
      '-lXrandr',
      '-lXtst',
      '-lcrypto',
    ],
    { stdio: 'inherit' },
  )
  console.log('Ubuntu X11 screen/input bridge compiled.')
  process.exit(0)
}
if (process.platform !== 'darwin') {
  console.log('Windows 使用随附的原生 DXVA2 PowerShell 桥，无需编译。')
  process.exit(0)
}
const at = process.argv.indexOf('--arch')
const { arch, cpu } = macArchitecture(at < 0 ? process.arch : process.argv[at + 1])
const destAt = process.argv.indexOf('--dest')
const destination = resolve(destAt < 0 ? `resources/bin/${arch}` : process.argv[destAt + 1])
mkdirSync(destination, { recursive: true })
mkdirSync('.native-cache', { recursive: true })
const cache = resolve('.native-cache')
const target = `${cpu}-apple-macosx14.0`
execFileSync(
  'clang',
  [
    '-fmodules',
    '-target',
    target,
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
    ...(arch === 'arm64'
      ? [
          'resources/vendor/m1ddc/sources/i2c.m',
          'resources/vendor/m1ddc/sources/ioregistry.m',
          'resources/vendor/m1ddc/sources/m1ddc.m',
        ]
      : ['-fobjc-arc', '-framework', 'AppKit', 'native/mac-intel-ddc.m']),
    '-o',
    resolve(destination, arch === 'arm64' ? 'm1ddc' : 'intel-ddc'),
  ],
  { stdio: 'inherit' },
)
execFileSync(
  'swiftc',
  [
    '-target',
    target,
    '-module-cache-path',
    cache,
    'native/mac-hotkey.swift',
    '-o',
    resolve(destination, 'mac-hotkey'),
  ],
  { stdio: 'inherit' },
)
execFileSync(
  'swiftc',
  [
    '-target',
    target,
    '-module-cache-path',
    cache,
    'native/mac-output.swift',
    '-o',
    resolve(destination, 'mac-output'),
  ],
  { stdio: 'inherit' },
)
verifyMacArchitecture(destination, arch)
console.log(`原生 DDC、快捷键和显示输出桥已编译并校验架构：${arch}。Intel 软断开仍未启用。`)
