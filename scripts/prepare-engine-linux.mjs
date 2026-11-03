// Build the pinned Deskflow source against this Ubuntu baseline. Qt >= 6.7
// must be installed first (CI uses Qt 6.8.3). Never reuse a newer distro's ELF.
import { execFileSync } from 'node:child_process'
import { cpSync, existsSync, mkdirSync, readdirSync, rmSync, writeFileSync } from 'node:fs'
import { join, basename } from 'node:path'

export function prepareLinux({
  root,
  cache,
  destination,
  source,
  sourceRevision,
  version,
  digest,
  download,
}) {
  if (process.platform !== 'linux')
    throw new Error(
      'Ubuntu engine preparation must run on Ubuntu, not through a macOS cross build.',
    )
  const sourceHash = digest(source)
  const buildRoot = join(cache, `linux-${process.arch}-${sourceHash.slice(0, 16)}`)
  const prefix = join(buildRoot, 'deps')
  mkdirSync(buildRoot, { recursive: true })
  const env = {
    ...process.env,
    PKG_CONFIG_PATH: [join(prefix, 'lib/pkgconfig'), process.env.PKG_CONFIG_PATH]
      .filter(Boolean)
      .join(':'),
  }
  const run = (tool, args, options = {}) =>
    execFileSync(tool, args, { env, stdio: 'inherit', ...options })
  const dependencies = [
    {
      name: 'libei',
      version: '1.3.0',
      sha: '162dc7b0d86a4575cdde1d3cb04f364f89a8b1ba6d95fe0b442564e0444d851d',
      url: 'https://gitlab.freedesktop.org/libinput/libei/-/archive/1.3.0/libei-1.3.0.tar.gz',
      options: ['-Dtests=disabled', '-Dliboeffis=disabled'],
    },
    {
      name: 'libportal',
      version: '0.9.1',
      sha: 'ea422b789ae487e04194d387bea031fd7485bf88a18aef8c767f7d1c29496a4e',
      url: 'https://codeload.github.com/flatpak/libportal/tar.gz/refs/tags/0.9.1',
      options: [
        '-Dbackend-gtk3=disabled',
        '-Dbackend-gtk4=disabled',
        '-Dbackend-qt5=disabled',
        '-Dbackend-qt6=disabled',
        '-Dintrospection=false',
        '-Dvapi=false',
        '-Ddocs=false',
        '-Dtests=false',
      ],
    },
  ]
  const target = join(destination, 'deskflow')
  const payload = join(buildRoot, 'payload')
  rmSync(payload, { recursive: true, force: true })
  mkdirSync(join(payload, 'runtime/lib'), { recursive: true })
  mkdirSync(join(payload, 'licenses'), { recursive: true })
  for (const dep of dependencies) {
    const archive = download(`${dep.name}-${dep.version}.tar.gz`, dep.url, dep.sha)
    const dir = join(buildRoot, `${dep.name}-${dep.version}`)
    if (!existsSync(dir)) run('tar', ['-xzf', archive, '-C', buildRoot])
    const build = join(buildRoot, `${dep.name}-build`)
    if (!existsSync(join(build, 'build.ninja')))
      run('meson', [
        'setup',
        build,
        dir,
        '--prefix',
        prefix,
        '--libdir',
        'lib',
        '--buildtype',
        'release',
        ...dep.options,
      ])
    run('meson', ['compile', '-C', build])
    run('meson', ['install', '-C', build])
    cpSync(archive, join(payload, `${dep.name}-${dep.version}-source.tar.gz`))
    for (const file of readdirSync(dir))
      if (/^(LICENSE|COPYING)/.test(file))
        cpSync(join(dir, file), join(payload, 'licenses', `${dep.name}-${file}`))
  }
  run('tar', ['-xzf', source, '-C', buildRoot])
  const sourceDir = join(buildRoot, `deskflow-${sourceRevision}`)
  const build = join(buildRoot, 'build')
  run('cmake', [
    '-S',
    sourceDir,
    '-B',
    build,
    '-G',
    'Ninja',
    '-DCMAKE_BUILD_TYPE=Release',
    '-DBUILD_TESTS=OFF',
    '-DBUILD_INSTALLER=OFF',
    '-DBUILD_X11_SUPPORT=ON',
    '-DCMAKE_BUILD_RPATH_USE_ORIGIN=ON',
  ])
  run('cmake', ['--build', build, '--target', 'deskflow-core', '--parallel', '2'])
  const runtime = join(payload, 'runtime')
  cpSync(join(build, 'bin/deskflow-core'), join(runtime, 'deskflow-core'))
  let qtpaths = 'qtpaths6'
  try {
    run(qtpaths, ['--query', 'QT_VERSION'], { stdio: 'pipe' })
  } catch {
    qtpaths = 'qtpaths'
  }
  const query = (name) =>
    run(qtpaths, ['--query', name], { encoding: 'utf8', stdio: 'pipe' }).trim()
  const qtPrefix = query('QT_INSTALL_PREFIX')
  const qtVersion = query('QT_VERSION')
  if (qtVersion !== '6.8.3')
    throw new Error(`This source recipe requires Qt 6.8.3, found ${qtVersion}`)
  const plugins = query('QT_INSTALL_PLUGINS')
  env.LD_LIBRARY_PATH = [join(prefix, 'lib'), query('QT_INSTALL_LIBS'), process.env.LD_LIBRARY_PATH]
    .filter(Boolean)
    .join(':')
  mkdirSync(join(runtime, 'plugins/platforms'), { recursive: true })
  const platformPlugins = ['libqxcb.so', 'libqoffscreen.so']
  for (const plugin of platformPlugins)
    cpSync(join(plugins, 'platforms', plugin), join(runtime, 'plugins/platforms', plugin))
  writeFileSync(join(runtime, 'qt.conf'), '[Paths]\nPlugins=plugins\n')
  // Bundle only privately built dependencies and Qt; distro libraries remain
  // package dependencies. Resolve recursively, including dlopened XCB plugin.
  const pending = [
    join(runtime, 'deskflow-core'),
    ...platformPlugins.map((name) => join(runtime, 'plugins/platforms', name)),
  ]
  const copied = new Set()
  const inventory = []
  while (pending.length) {
    const binary = pending.shift()
    const output = run('ldd', [binary], { encoding: 'utf8', stdio: 'pipe' })
    if (output.includes('not found')) throw new Error(`Unresolved engine dependencies:\n${output}`)
    for (const match of output.matchAll(/=>\s+(\/\S+)/g)) {
      const path = match[1]
      if (
        !(
          path.startsWith(prefix + '/') ||
          (path.startsWith(qtPrefix + '/') && /^lib(Qt6|icu)/.test(basename(path)))
        )
      )
        continue
      const name = basename(path)
      if (copied.has(name)) continue
      copied.add(name)
      const target = join(runtime, 'lib', name)
      cpSync(path, target)
      pending.push(target)
      inventory.push(name)
    }
  }
  for (const binary of [
    join(runtime, 'deskflow-core'),
    ...platformPlugins.map((name) => join(runtime, 'plugins/platforms', name)),
    ...[...copied].map((name) => join(runtime, 'lib', name)),
  ]) {
    const rpath = binary.endsWith('/deskflow-core')
      ? '$ORIGIN/lib'
      : binary.includes('/plugins/')
        ? '$ORIGIN/../../lib'
        : '$ORIGIN'
    run('patchelf', ['--set-rpath', rpath, binary])
  }
  const isolatedEnv = {
    ...env,
    LD_LIBRARY_PATH: '',
    QT_PLUGIN_PATH: '',
    QT_QPA_PLATFORM: 'offscreen',
  }
  for (const binary of [
    join(runtime, 'deskflow-core'),
    ...platformPlugins.map((name) => join(runtime, 'plugins/platforms', name)),
  ]) {
    const linked = run('ldd', [binary], { encoding: 'utf8', stdio: 'pipe', env: isolatedEnv })
    if (
      linked.includes('not found') ||
      linked.includes(prefix + '/') ||
      (qtPrefix !== '/usr' && linked.includes(qtPrefix + '/'))
    ) {
      throw new Error(`Runtime is not independent of the build SDK:\n${linked}`)
    }
  }
  const output = run(join(runtime, 'deskflow-core'), ['--version'], {
    encoding: 'utf8',
    stdio: 'pipe',
    env: isolatedEnv,
  })
  if (!output.includes(version)) throw new Error(`Unexpected engine version: ${output}`)
  // All bundled Qt modules come from qtbase. Distribute its exact source and
  // license texts alongside the dynamically replaceable libraries.
  const qtSource = download(
    'qtbase-everywhere-src-6.8.3.tar.xz',
    'https://download.qt.io/archive/qt/6.8/6.8.3/submodules/qtbase-everywhere-src-6.8.3.tar.xz',
    '56001b905601bb9023d399f3ba780d7fa940f3e4861e496a7c490331f49e0b80',
  )
  cpSync(qtSource, join(payload, 'qtbase-everywhere-src-6.8.3.tar.xz'))
  run('tar', ['-xJf', qtSource, '-C', buildRoot, 'qtbase-everywhere-src-6.8.3/LICENSES'])
  cpSync(
    join(buildRoot, 'qtbase-everywhere-src-6.8.3/LICENSES'),
    join(payload, 'licenses/qtbase'),
    { recursive: true },
  )
  // Preserve any additional SDK license texts as well.
  const qtLicenses = [join(qtPrefix, 'licenses'), join(qtPrefix, '../../Licenses')].find(existsSync)
  if (qtLicenses) cpSync(qtLicenses, join(payload, 'licenses/qt'), { recursive: true })
  writeFileSync(
    join(payload, 'licenses/LINUX-RUNTIME.txt'),
    `Deskflow ${version}, source ${sourceRevision}\nPrivate runtime libraries: ${inventory.join(', ')}\nQt 6.8.3 is dynamically linked; qtbase corresponding source and license texts are included. Qt libraries can be replaced in the engine cache.\nlibei and libportal corresponding source archives are included.\n`,
  )
  cpSync(source, join(payload, `deskflow-${version}-source.tar.gz`))
  cpSync(join(sourceDir, 'LICENSES'), join(payload, 'licenses'), { recursive: true })
  cpSync(join(sourceDir, 'LICENSE'), join(payload, 'licenses/DESKFLOW-LICENSE'))
  const binaryHash = digest(join(runtime, 'deskflow-core'))
  run('tar', ['-czf', join(payload, 'runtime.tar.gz'), '-C', payload, 'runtime'])
  const archiveHash = digest(join(payload, 'runtime.tar.gz'))
  rmSync(runtime, { recursive: true })
  writeFileSync(
    join(payload, 'manifest.json'),
    JSON.stringify(
      {
        version,
        platform: 'linux',
        arch: process.arch,
        binary: 'runtime/deskflow-core',
        sha256: binaryHash,
        archiveSha256: archiveHash,
        sourceRevision,
        sourceSha256: sourceHash,
        // Source builds key the installed runtime by its complete content.
        upstreamArchiveSha256: archiveHash,
      },
      null,
      2,
    ) + '\n',
  )
  rmSync(target, { recursive: true, force: true })
  cpSync(payload, target, { recursive: true })
  run(process.execPath, [join(root, 'scripts/build-native.mjs')], { cwd: root })
  mkdirSync(join(destination, 'native/bin'), { recursive: true })
  cpSync(join(root, 'resources/bin/linux-input'), join(destination, 'native/bin/linux-input'))
  for (const file of ['THIRD_PARTY_NOTICES.md', 'LICENSE'])
    cpSync(join(root, file), join(destination, file))
  cpSync(join(root, 'resources/JS-NOTICES.txt'), join(destination, 'JS-NOTICES.txt'))
  console.log(`Prepared Deskflow ${version} from verified source for Ubuntu/${process.arch}`)
}
