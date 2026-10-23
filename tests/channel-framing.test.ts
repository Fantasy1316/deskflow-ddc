import { describe, expect, it } from 'vitest'
import { EventEmitter } from 'node:events'
import { randomBytes } from 'node:crypto'
import type { Socket } from 'node:net'
import { SecureChannel, SessionCodec } from '../electron/core/channel'

class TestSocket extends EventEmitter {
  destroyed = false
  writes: string[] = []
  setNoDelay() {}
  setKeepAlive() {}
  setEncoding() {}
  write(data: string) {
    this.writes.push(data)
  }
  destroy() {
    if (!this.destroyed) {
      this.destroyed = true
      this.emit('close')
    }
  }
}
describe('control stream framing independent of TCP chunks', () => {
  function setup() {
    const socket = new TestSocket(),
      key = randomBytes(32),
      nonce = randomBytes(32).toString('hex')
    const channel = new SecureChannel(socket as unknown as Socket, key, true),
      faults: string[] = []
    channel.on('fault', (message) => faults.push(message))
    socket.emit('data', JSON.stringify({ kind: 'hello', version: 1, nonce }) + '\n')
    const challenge = JSON.parse(socket.writes[0]!)
    const codec = new SessionCodec(key, nonce, challenge.nonce, false)
    socket.emit('data', JSON.stringify(codec.seal({ kind: 'auth' })) + '\n')
    return { socket, channel, codec, faults }
  }
  it('accepts multiple valid frames larger than the aggregate buffer limit and split UTF-8 payloads', () => {
    const { socket, channel, codec, faults } = setup(),
      events: unknown[] = []
    channel.on('event', (_name, data) => events.push(data))
    const frame = () =>
      JSON.stringify(codec.seal({ kind: 'event', name: 'sample', payload: '图'.repeat(15000) })) +
      '\n'
    try {
      const a = frame(),
        b = frame(),
        c = frame()
      socket.emit('data', a.slice(0, 100))
      socket.emit('data', a.slice(100) + b + c)
      expect(events).toHaveLength(3)
      expect(faults).toEqual([])
      expect(channel.ready).toBe(true)
    } finally {
      channel.close()
    }
  })
  it.each([false, true])('rejects an oversized individual line, terminated=%s', (terminated) => {
    const { socket, channel, faults } = setup()
    socket.emit('data', 'x'.repeat(128 * 1024 + 1) + (terminated ? '\n' : ''))
    expect(faults).toContain('控制消息超出大小限制')
    expect(channel.ready).toBe(false)
  })
})
