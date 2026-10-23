import { afterEach, describe, expect, it } from 'vitest'
import { randomBytes } from 'node:crypto'
import { createServer, connect, type Server } from 'node:net'
import {
  SecureChannel,
  SessionCodec,
  decodePairingCode,
  newPairingCode,
} from '../electron/core/channel'

const channels: SecureChannel[] = [],
  servers: Server[] = []
afterEach(async () => {
  channels.splice(0).forEach((c) => c.close())
  await Promise.all(servers.splice(0).map((s) => new Promise<void>((r) => s.close(() => r()))))
})
const randomNonce = () => randomBytes(32).toString('hex')
describe('session authentication', () => {
  it('encrypts each direction, rejects tampering and replay', () => {
    const key = randomBytes(32),
      a = randomNonce(),
      b = randomNonce()
    const host = new SessionCodec(key, a, b, true),
      client = new SessionCodec(key, a, b, false)
    const frame = host.seal({ message: '输入源切换', input: 3 })
    expect(JSON.stringify(frame)).not.toContain('输入源')
    expect(client.open(frame)).toEqual({ message: '输入源切换', input: 3 })
    expect(() => client.open(frame)).toThrow(/序号/)
    const next = host.seal('hello')
    next.tag = Buffer.alloc(16).toString('base64')
    expect(() => client.open(next)).toThrow()
    expect(host.open(client.seal({ input: 1 }))).toEqual({ input: 1 })
  })
  it('uses fresh keys across reconnects and rejects a wrong pairing secret', () => {
    const key = randomBytes(32),
      a = randomNonce(),
      b = randomNonce()
    const packet = new SessionCodec(key, a, b, true).seal('private')
    expect(() => new SessionCodec(randomBytes(32), a, b, false).open(packet)).toThrow()
    expect(() => new SessionCodec(key, randomNonce(), randomNonce(), false).open(packet)).toThrow()
  })
  it('accepts only complete canonical 256-bit pairing codes', () => {
    expect(decodePairingCode(newPairingCode())).toHaveLength(32)
    expect(() => decodePairingCode('123456')).toThrow()
    expect(() => decodePairingCode('SB1-' + 'a'.repeat(44))).toThrow()
  })
})
describe('real TCP control connection', () => {
  it('performs the authenticated handshake, RPC and remote errors', async () => {
    const key = randomBytes(32)
    let host!: SecureChannel
    const hostReady = new Promise<void>((resolve) => {
      const server = createServer((socket) => {
        host = new SecureChannel(socket, key, true)
        channels.push(host)
        host.on('fault', () => {})
        host.onRequest(async (method, payload) => {
          if (method === 'fail') throw new Error('目标未连接')
          if (method === 'never') return new Promise(() => {})
          return { method, payload }
        })
        host.once('ready', () => resolve())
      })
      servers.push(server)
      server.listen(0, '127.0.0.1')
    })
    const server = servers[0]!
    if (!server.listening) await new Promise<void>((resolve) => server.once('listening', resolve))
    const address = server.address() as { port: number }
    const client = new SecureChannel(connect(address.port, '127.0.0.1'), key, false)
    channels.push(client)
    await Promise.all([
      hostReady,
      new Promise<void>((resolve, reject) => {
        client.once('ready', resolve)
        client.once('fault', (message) => reject(new Error(message)))
      }),
    ])
    expect(await client.request('set-input', { input: 3 })).toEqual({
      method: 'set-input',
      payload: { input: 3 },
    })
    await expect(client.request('fail')).rejects.toThrow('目标未连接')
    const waiting = client.request('never', null, 1)
    await expect(waiting).rejects.toThrow(/超时/)
  })
  it('rejects a client with the wrong pairing code before any RPC', async () => {
    const server = createServer((socket) => {
      const channel = new SecureChannel(socket, randomBytes(32), true)
      channels.push(channel)
      channel.on('fault', () => {})
    })
    servers.push(server)
    await new Promise<void>((r) => server.listen(0, '127.0.0.1', r))
    const client = new SecureChannel(
      connect((server.address() as { port: number }).port, '127.0.0.1'),
      randomBytes(32),
      false,
    )
    channels.push(client)
    let authenticated = false
    client.on('ready', () => {
      authenticated = true
    })
    await new Promise<void>((r) => client.once('closed', r))
    expect(authenticated).toBe(false)
    await expect(client.request('set-input', 1)).rejects.toThrow(/尚未连接/)
  })
})
