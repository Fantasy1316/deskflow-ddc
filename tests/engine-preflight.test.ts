import { afterEach, describe, expect, it } from 'vitest'
import { createServer, type Server } from 'node:net'
import { randomUUID } from 'node:crypto'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { assertNoRunningEngine, assertPortAvailable } from '../electron/core/engine-preflight'

const servers: Server[] = []
afterEach(async () => {
  await Promise.all(
    servers
      .splice(0)
      .map((server) => new Promise<void>((resolve) => server.close(() => resolve()))),
  )
})
describe('isolating Deskflow DDC from an existing Deskflow process', () => {
  it('rejects a live IPC endpoint without sending any command to that engine', async () => {
    const path =
      process.platform === 'win32'
        ? `\\\\.\\pipe\\screenbridge-test-${randomUUID()}`
        : join(tmpdir(), `sb-test-${randomUUID()}`)
    let received = ''
    const server = createServer((socket) => {
      socket.on('data', (data) => {
        received += data
      })
      socket.on('error', () => {})
    })
    servers.push(server)
    await new Promise<void>((resolve) => server.listen(path, resolve))
    await expect(assertNoRunningEngine(path)).rejects.toThrow('已运行的 Deskflow')
    expect(received).toBe('')
  })
  it('allows startup when no IPC listener exists', async () => {
    const path =
      process.platform === 'win32'
        ? `\\\\.\\pipe\\screenbridge-test-${randomUUID()}`
        : join(tmpdir(), `sb-test-${randomUUID()}`)
    await expect(assertNoRunningEngine(path)).resolves.toBeUndefined()
  })
  it('rejects an occupied server port and permits it after the listener closes', async () => {
    const server = createServer()
    servers.push(server)
    await new Promise<void>((resolve) => server.listen(0, '0.0.0.0', resolve))
    const port = (server.address() as { port: number }).port
    await expect(assertPortAvailable(port)).rejects.toThrow(`${port} 已被占用`)
    await new Promise<void>((resolve) => server.close(() => resolve()))
    await expect(assertPortAvailable(port)).resolves.toBeUndefined()
  })
})
