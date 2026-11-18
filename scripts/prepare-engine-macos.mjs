import { execFileSync } from 'node:child_process'
import {
  cpSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import { createHash } from 'node:crypto'
import { join } from 'node:path'
import { patchEngine } from './patch-engine.mjs'
import { downloadVerified } from './download-verified.mjs'

export function prepareMacCore({ root, cache, source, sourceRevision, target, arch }) {
  const files = [
    'scripts/patch-engine.mjs',
    'scripts/prepare-engine-macos.mjs',
    'resources/vendor/deskflow/primary-entry.h',
    'resources/vendor/deskflow/primary-route.h',
  ]
  const patchSha256 = createHash('sha256')
    .update(Buffer.concat(files.map((name) => readFileSync(join(root, name)))))
    .digest('hex')
  const buildRoot = join(cache, `mac-core-${arch}-${patchSha256.slice(0, 16)}`)
  mkdirSync(buildRoot, { recursive: true })
  const run = (tool, args, options = {}) =>
    execFileSync(tool, args, { stdio: 'inherit', ...options })
  const download = (name, url, sha256) => downloadVerified({ cache, name, url, sha256 })
  const cpu = arch === 'arm64' ? 'arm64' : 'x86_64'
  const qtVersion = execFileSync('qmake', ['-query', 'QT_VERSION'], { encoding: 'utf8' }).trim()
  if (qtVersion !== '6.8.3')
    throw new Error(`Pinned macOS core requires Qt 6.8.3, found ${qtVersion}`)
  const qtPrefix = execFileSync('qmake', ['-query', 'QT_INSTALL_PREFIX'], {
    encoding: 'utf8',
  }).trim()
  // Homebrew's static OpenSSL may require a newer macOS than our application.
  // Compile the pinned LTS source for the same CPU and macOS 14 baseline.
  const opensslVersion = '3.5.9'
  const opensslSource = download(
    `openssl-${opensslVersion}.tar.gz`,
    `https://github.com/openssl/openssl/releases/download/openssl-${opensslVersion}/openssl-${opensslVersion}.tar.gz`,
    '603f5602e2eef00d77fbd429d34dcd5822bb301757a1bc9cdb24c670f1eb859a',
  )
  const sslRoot = join(cache, `openssl-${opensslVersion}-${cpu}-macos14`)
  const sslPrefix = join(sslRoot, 'install')
  if (!existsSync(join(sslPrefix, '.complete'))) {
    rmSync(sslRoot, { recursive: true, force: true })
    mkdirSync(sslRoot, { recursive: true })
    run('tar', ['-xzf', opensslSource, '-C', sslRoot])
    const cwd = join(sslRoot, `openssl-${opensslVersion}`)
    run(
      'perl',
      [
        'Configure',
        `darwin64-${cpu}-cc`,
        'no-shared',
        'no-tests',
        'no-module',
        `--prefix=${sslPrefix}`,
        '--libdir=lib',
        '-mmacosx-version-min=14.0',
      ],
      { cwd },
    )
    run('make', ['-j4', 'build_sw'], { cwd })
    run('make', ['install_sw'], { cwd })
    writeFileSync(join(sslPrefix, '.complete'), opensslVersion)
  }
  run('tar', ['-xzf', source, '-C', buildRoot])
  const directory = join(buildRoot, `deskflow-${sourceRevision}`)
  patchEngine(directory, root)
  const build = join(buildRoot, 'build')
  run('cmake', [
    '-S',
    directory,
    '-B',
    build,
    '-G',
    'Ninja',
    '-DCMAKE_BUILD_TYPE=Release',
    '-DBUILD_TESTS=OFF',
    '-DBUILD_INSTALLER=OFF',
    '-DBUILD_OSX_BUNDLE=OFF',
    `-DCMAKE_OSX_ARCHITECTURES=${cpu}`,
    '-DCMAKE_OSX_DEPLOYMENT_TARGET=14.0',
    `-DCMAKE_PREFIX_PATH=${qtPrefix}`,
    `-DOPENSSL_ROOT_DIR=${sslPrefix}`,
    `-DOPENSSL_INCLUDE_DIR=${join(sslPrefix, 'include')}`,
    `-DOPENSSL_SSL_LIBRARY=${join(sslPrefix, 'lib/libssl.a')}`,
    `-DOPENSSL_CRYPTO_LIBRARY=${join(sslPrefix, 'lib/libcrypto.a')}`,
  ])
  run('cmake', ['--build', build, '--target', 'deskflow-core', '--parallel', '4'])
  // Package a private core-only bundle. Deploy the same Qt that compiled it;
  // never mix SDK versions with the upstream GUI's frameworks.
  const app = join(target, 'Deskflow.app')
  rmSync(app, { recursive: true, force: true })
  mkdirSync(join(app, 'Contents/MacOS'), { recursive: true })
  run('cp', [join(build, 'bin/deskflow-core'), join(app, 'Contents/MacOS/deskflow-core')])
  run('/usr/libexec/PlistBuddy', [
    '-c',
    'Add :CFBundleExecutable string deskflow-core',
    join(app, 'Contents/Info.plist'),
  ])
  run('/usr/libexec/PlistBuddy', [
    '-c',
    'Add :CFBundleIdentifier string org.deskflow.deskflow.core',
    join(app, 'Contents/Info.plist'),
  ])
  run('/usr/libexec/PlistBuddy', [
    '-c',
    'Add :CFBundlePackageType string APPL',
    join(app, 'Contents/Info.plist'),
  ])
  for (const [name, value] of [
    ['CFBundleName', 'Deskflow DDC Input Engine'],
    ['CFBundleShortVersionString', '1.27.0'],
    ['CFBundleVersion', '1.27.0'],
    [
      'NSLocalNetworkUsageDescription',
      'Connect to your paired computer for keyboard and mouse sharing.',
    ],
  ])
    run('/usr/libexec/PlistBuddy', [
      '-c',
      `Add :${name} string ${value}`,
      join(app, 'Contents/Info.plist'),
    ])
  run(join(qtPrefix, 'bin/macdeployqt'), [
    app,
    '-always-overwrite',
    '-no-strip',
    `-executable=${join(app, 'Contents/MacOS/deskflow-core')}`,
  ])
  run('codesign', ['--force', '--deep', '--sign', '-', app])
  const binary = join(app, 'Contents/MacOS/deskflow-core')
  const linked = execFileSync('otool', ['-L', binary], { encoding: 'utf8' })
  if (/\n\s*\/(?:opt|Users|usr\/local)\//.test(linked))
    throw new Error(`Core depends on build-machine libraries:\n${linked}`)
  const version = execFileSync(binary, ['--version'], { encoding: 'utf8' })
  if (!version.includes('1.27.0')) throw new Error(`Unexpected core: ${version}`)
  // Include all changes needed to reproduce the derivative binary beside the
  // pinned upstream source archive and notices.
  mkdirSync(join(target, 'ddc-patches'), { recursive: true })
  for (const file of files)
    copyFileSync(join(root, file), join(target, 'ddc-patches', file.split('/').pop()))
  for (const name of ['download-verified.mjs'])
    copyFileSync(join(root, 'scripts', name), join(target, 'ddc-patches', name))
  const qtSource = download(
    'qtbase-everywhere-src-6.8.3.tar.xz',
    'https://download.qt.io/archive/qt/6.8/6.8.3/submodules/qtbase-everywhere-src-6.8.3.tar.xz',
    '56001b905601bb9023d399f3ba780d7fa940f3e4861e496a7c490331f49e0b80',
  )
  copyFileSync(qtSource, join(target, 'qtbase-everywhere-src-6.8.3.tar.xz'))
  run('tar', ['-xJf', qtSource, '-C', buildRoot, 'qtbase-everywhere-src-6.8.3/LICENSES'])
  cpSync(join(buildRoot, 'qtbase-everywhere-src-6.8.3/LICENSES'), join(target, 'licenses/qtbase'), {
    recursive: true,
  })
  copyFileSync(opensslSource, join(target, `openssl-${opensslVersion}-source.tar.gz`))
  cpSync(
    join(sslRoot, `openssl-${opensslVersion}/LICENSE.txt`),
    join(target, 'licenses/OPENSSL-LICENSE.txt'),
  )
  writeFileSync(
    join(target, 'ddc-patches/README.txt'),
    'Modified Deskflow core: primary entry mapping, ordered primary cursor capture, unchanged-option reload suppression, transport diagnostics.\n' +
      'Build recipe and all changed headers/scripts are included here. Original source, Qt 6.8.3 source and OpenSSL source are alongside this directory.\n' +
      'Reproduce from the project source with Node.js, CMake, Ninja, the Qt 6.8.3 macOS SDK and pnpm prepare:engine.\n' +
      'Qt is dynamically linked; its libraries may be replaced in the private engine cache.\n',
  )
  return {
    patchSha256,
    sourceDirectory: directory,
    qtVersion,
    opensslVersion,
  }
}
