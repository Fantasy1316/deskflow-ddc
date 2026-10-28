import { execFileSync } from 'node:child_process'
import { readFileSync, mkdtempSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

if (process.platform !== 'win32') throw new Error('Run this native test on Windows.')
const bridge = readFileSync('native/windows/monitor.ps1', 'utf8')
const source = bridge.match(/Add-Type -TypeDefinition @'\r?\n([\s\S]*?)\r?\n'@/)
if (!source) throw new Error('Cannot locate the native C# bindings.')
const temp = mkdtempSync(join(tmpdir(), 'ddc-screen-test-'))
try {
  writeFileSync(
    join(temp, 'test.cs'),
    source[1] + '\n' + readFileSync('native/tests/windows-screen-identity.cs', 'utf8'),
  )
  writeFileSync(
    join(temp, 'run.ps1'),
    "$ErrorActionPreference = 'Stop'\nAdd-Type -Path (Join-Path $PSScriptRoot 'test.cs')\n[WindowsScreenIdentityTests]::Main()\n",
  )
  execFileSync(
    'powershell.exe',
    ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', join(temp, 'run.ps1')],
    { stdio: 'inherit', windowsHide: true },
  )
} finally {
  rmSync(temp, { recursive: true, force: true })
}
