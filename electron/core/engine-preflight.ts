import { connect, createServer } from 'node:net'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

export function deskflowIpcPath(tempDirectory = tmpdir(), platform = process.platform) {
  return platform === 'win32' ? '\\\\.\\pipe\\deskflow-core' : join(tempDirectory, 'deskflow-core')
}
export async function assertNoRunningEngine(path = deskflowIpcPath()) {
  await new Promise<void>((resolve, reject) => {
    const socket = connect(path)
    const finish = (error?: Error) => {
      socket.destroy()
      error ? reject(error) : resolve()
    }
    socket.setTimeout(1000, () =>
      finish(new Error('无法确认 Deskflow 是否仍在运行，请退出独立 Deskflow 后重试')),
    )
    socket.once('connect', () =>
      finish(
        new Error(
          '检测到已运行的 Deskflow 引擎。请先在两端完整退出独立 Deskflow（包括托盘或后台服务），再启动 Deskflow DDC 共享。Deskflow DDC 不会接管已有引擎。',
        ),
      ),
    )
    socket.once('error', (error: NodeJS.ErrnoException) => {
      if (error.code === 'ENOENT' || error.code === 'ECONNREFUSED') finish()
      else finish(new Error(`无法检查 Deskflow 引擎：${error.code ?? error.message}`))
    })
  })
}
export async function assertPortAvailable(port: number) {
  await new Promise<void>((resolve, reject) => {
    const server = createServer()
    server.once('error', (error: NodeJS.ErrnoException) =>
      reject(
        new Error(
          error.code === 'EADDRINUSE'
            ? `键鼠端口 ${port} 已被占用。请先退出独立 Deskflow 或停止占用该端口的程序，再启动 Deskflow DDC 共享。`
            : `无法使用键鼠端口 ${port}：${error.code ?? error.message}`,
        ),
      ),
    )
    server.listen({ port, host: '0.0.0.0', exclusive: true }, () =>
      server.close((error) => (error ? reject(error) : resolve())),
    )
  })
}
