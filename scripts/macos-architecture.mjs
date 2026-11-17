import { execFileSync } from 'node:child_process'
import { openSync, readSync, closeSync, readdirSync } from 'node:fs'
import { join } from 'node:path'

export function macArchitecture(arch) {
  if (arch === 'arm64' || arch === 'aarch64') return { arch: 'arm64', cpu: 'arm64' }
  if (arch === 'x64' || arch === 'x86_64') return { arch: 'x64', cpu: 'x86_64' }
  throw new Error(`Unsupported macOS architecture: ${arch}`)
}

export function isMachO(header) {
  return [
    'feedface',
    'cefaedfe',
    'feedfacf',
    'cffaedfe',
    'cafebabe',
    'bebafeca',
    'cafebabf',
    'bfbafeca',
  ].includes(header.subarray(0, 4).toString('hex'))
}

// Inspect every real executable/library, including Qt plugins and frameworks.
// Never execute an application here: preparing dependencies must not start sharing.
export function verifyMacArchitecture(directory, arch) {
  const { cpu } = macArchitecture(arch)
  let count = 0
  function visit(root) {
    for (const entry of readdirSync(root, { withFileTypes: true })) {
      const path = join(root, entry.name)
      if (entry.isDirectory()) visit(path)
      else if (entry.isFile()) {
        const header = Buffer.alloc(4)
        const file = openSync(path, 'r')
        try {
          readSync(file, header, 0, 4, 0)
        } finally {
          closeSync(file)
        }
        if (!isMachO(header)) continue
        execFileSync('lipo', [path, '-verify_arch', cpu], { stdio: 'pipe' })
        count++
      }
    }
  }
  visit(directory)
  if (!count) throw new Error(`No macOS executables found in ${directory}`)
  return count
}
