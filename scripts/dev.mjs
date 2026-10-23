import { spawn } from 'node:child_process'
import { existsSync } from 'node:fs'
const run = (command, args, options = {}) =>
  spawn(command, args, { stdio: 'inherit', shell: process.platform === 'win32', ...options })
const compile = run('pnpm', ['exec', 'tsc', '-p', 'tsconfig.electron.json'])
await new Promise((resolve, reject) =>
  compile.on('exit', (code) => (code === 0 ? resolve() : reject(new Error('桌面端编译失败')))),
)
if (process.platform === 'darwin' && !existsSync('resources/bin/m1ddc')) {
  const native = run('node', ['scripts/build-native.mjs'])
  await new Promise((resolve, reject) =>
    native.on('exit', (code) => (code === 0 ? resolve() : reject(new Error('原生程序编译失败')))),
  )
}
const vite = run('pnpm', ['exec', 'vite', '--host', '127.0.0.1'])
let desktop
const cleanup = () => {
  desktop?.kill()
  vite.kill()
}
process.on('SIGINT', cleanup)
process.on('SIGTERM', cleanup)
for (let attempt = 0; attempt < 80; attempt++) {
  try {
    const response = await fetch('http://127.0.0.1:5173')
    if (response.ok) break
  } catch {}
  await new Promise((resolve) => setTimeout(resolve, 250))
}
desktop = run('pnpm', ['exec', 'electron', '.'], {
  env: { ...process.env, DESKFLOW_DDC_DEV_URL: 'http://127.0.0.1:5173' },
})
desktop.on('exit', () => {
  vite.kill()
  process.exit()
})
