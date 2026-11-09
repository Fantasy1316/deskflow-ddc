import { execFileSync } from 'node:child_process'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'

// Compile both hooks without running an installer or touching installed services.
const compiler = process.argv[2] || process.env.MAKENSIS || 'makensis'
const directory = mkdtempSync(join(tmpdir(), 'ddc-installer-'))
const quote = (value) => value.replaceAll('$', '$$').replaceAll('"', '$\\"')
try {
  for (const loader of [false, true]) {
    const stage = join(directory, loader ? 'GNU output with spaces' : 'MSVC output with spaces')
    mkdirSync(stage)
    writeFileSync(join(stage, 'deskflow-ddc.exe'), 'test executable payload')
    if (loader) writeFileSync(join(stage, 'WebView2Loader.dll'), 'test loader payload')
    const file = join(stage, 'test.nsi')
    writeFileSync(
      file,
      `Unicode true
Name "Deskflow installer regression"
OutFile "${quote(join(stage, 'installer.exe'))}"
RequestExecutionLevel user
!include LogicLib.nsh
!define MAINBINARYNAME "deskflow-ddc"
!define MAINBINARYSRCPATH "${quote(join(stage, 'deskflow-ddc.exe'))}"
!include "${quote(resolve('native/windows/installer-hooks.nsh'))}"
Section
  !insertmacro NSIS_HOOK_PREINSTALL
  WriteUninstaller "$TEMP\\ddc-test-uninstall.exe"
SectionEnd
Section "Uninstall"
  !insertmacro NSIS_HOOK_PREUNINSTALL
SectionEnd
`,
    )
    execFileSync(compiler, ['-V2', file], { stdio: 'pipe' })
    const expanded = execFileSync(compiler, ['-PPO', file], { encoding: 'utf8' })
    const staged = expanded.indexOf('File "/oname=$PLUGINSDIR\\WebView2Loader.dll"')
    const cleanup = expanded.indexOf('ExecWait')
    if (loader !== staged >= 0) throw new Error('Cleanup loader staging does not match the build')
    if (loader && staged > cleanup)
      throw new Error('Cleanup executes before its loader is extracted')
  }
  console.log('NSIS cleanup hooks compiled for GNU and MSVC layouts, including paths with spaces.')
} finally {
  rmSync(directory, { recursive: true, force: true })
}
