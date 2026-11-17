import { test } from 'node:test'
import assert from 'node:assert/strict'
import { macArchitecture, isMachO, verifyMacArchitecture } from './macos-architecture.mjs'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { execFileSync } from 'node:child_process'

test('macOS aliases normalize to the manifest architecture and compiler target', () => {
  for (const arch of ['x64', 'x86_64'])
    assert.deepEqual(macArchitecture(arch), { arch: 'x64', cpu: 'x86_64' })
  for (const arch of ['arm64', 'aarch64'])
    assert.deepEqual(macArchitecture(arch), { arch: 'arm64', cpu: 'arm64' })
  for (const arch of [undefined, 'ia32', 'universal', ''])
    assert.throws(() => macArchitecture(arch))
})

test('thin and universal Mach-O are recognized without treating shell/text as executables', () => {
  for (const magic of ['cffaedfe', 'feedfacf', 'cafebabe', 'cafebabf'])
    assert.equal(isMachO(Buffer.from(magic, 'hex')), true)
  assert.equal(isMachO(Buffer.from('#!/b')), false)
  assert.equal(isMachO(Buffer.alloc(0)), false)
})

test('an empty payload is rejected instead of claiming dependency validation', () => {
  const root = mkdtempSync(join(tmpdir(), 'ddc-arch-'))
  try {
    writeFileSync(join(root, 'not-a-library'), 'text')
    assert.throws(() => verifyMacArchitecture(root, 'x64'), /No macOS executables/)
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test(
  'real Mach-O validation accepts the target and rejects a dependency of the wrong architecture',
  { skip: process.platform !== 'darwin' },
  () => {
    const root = mkdtempSync(join(tmpdir(), 'ddc-mach-o-'))
    try {
      execFileSync(
        'clang',
        ['-target', 'x86_64-apple-macosx14.0', '-x', 'c', '-', '-o', join(root, 'helper')],
        { input: 'int main(void) { return 0; }' },
      )
      assert.equal(verifyMacArchitecture(root, 'x64'), 1)
      assert.throws(() => verifyMacArchitecture(root, 'arm64'))
      execFileSync(
        'clang',
        ['-target', 'arm64-apple-macosx14.0', '-x', 'c', '-', '-o', join(root, 'dependency')],
        { input: 'int main(void) { return 0; }' },
      )
      assert.throws(() => verifyMacArchitecture(root, 'x64'))
    } finally {
      rmSync(root, { recursive: true, force: true })
    }
  },
)
