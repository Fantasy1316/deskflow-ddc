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
