import { execFileSync } from 'node:child_process'
import { mkdirSync } from 'node:fs'
mkdirSync('.native-cache', { recursive: true })
execFileSync(
  'clang',
  [
    '-std=c11',
    '-Wall',
    '-Wextra',
    '-Iresources/vendor/m1ddc/headers',
    'native/tests/ddc-reply.c',
    '-o',
    '.native-cache/test-ddc',
  ],
  { stdio: 'inherit' },
)
execFileSync('.native-cache/test-ddc', [], { stdio: 'inherit' })
if (process.platform === 'darwin') {
  execFileSync(
    'clang',
    [
      '-fobjc-arc',
      '-Wall',
      '-Wextra',
      '-framework',
      'AppKit',
      'native/tests/mac-lifecycle.m',
      '-o',
      '.native-cache/test-mac-lifecycle',
    ],
    { stdio: 'inherit' },
  )
  execFileSync('.native-cache/test-mac-lifecycle', [], { stdio: 'inherit' })
}
