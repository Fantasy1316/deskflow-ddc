import { createCipheriv, createDecipheriv, hkdfSync, randomBytes } from 'node:crypto'
import { EventEmitter } from 'node:events'
import type { Socket } from 'node:net'
import { z } from 'zod'

const MAX_FRAME = 128 * 1024
const nonceSchema = z.string().regex(/^[a-f0-9]{64}$/)
export function decodePairingCode(code: string): Buffer {
  const compact = code.trim().replace(/^SB1-/, '')
  if (!/^[A-Za-z0-9_-]{43}$/.test(compact))
    throw new Error('配对码格式不正确，请完整复制提供端的配对码')
  const key = Buffer.from(compact, 'base64url')
  if (key.length !== 32 || key.toString('base64url') !== compact)
    throw new Error('配对码格式不正确')
  return key
}
export function newPairingCode() {
  return `SB1-${randomBytes(32).toString('base64url')}`
}

/** A fresh random handshake salt and distinct directional keys prevent nonce reuse across sessions. */
export class SessionCodec {
  private sent = 0n
  private received = 0n
  private tx: Buffer
  private rx: Buffer
  constructor(key: Buffer, clientNonce: string, serverNonce: string, server: boolean) {
    nonceSchema.parse(clientNonce)
    nonceSchema.parse(serverNonce)
    const salt = Buffer.from(clientNonce + serverNonce, 'hex')
    const derive = (label: string) =>
      Buffer.from(hkdfSync('sha256', key, salt, `screenbridge-v1/${label}`, 36))
    this.tx = derive(server ? 'server-to-client' : 'client-to-server')
    this.rx = derive(server ? 'client-to-server' : 'server-to-client')
  }
  private iv(material: Buffer, sequence: bigint) {
    const iv = Buffer.alloc(12)
    material.copy(iv, 0, 32, 36)
    iv.writeBigUInt64BE(sequence, 4)
    return iv
  }
  seal(payload: unknown) {
    const plain = Buffer.from(JSON.stringify(payload))
    if (plain.length > MAX_FRAME / 2) throw new Error('控制消息过大')
    const seq = this.sent++
    const cipher = createCipheriv('aes-256-gcm', this.tx.subarray(0, 32), this.iv(this.tx, seq))
    cipher.setAAD(Buffer.from(`screenbridge-v1:${seq}`))
    const data = Buffer.concat([cipher.update(plain), cipher.final()])
    return {
      seq: String(seq),
      data: data.toString('base64'),
      tag: cipher.getAuthTag().toString('base64'),
    }
  }
  open(value: unknown): unknown {
    const frame = z
      .object({
        seq: z.string().regex(/^\d{1,20}$/),
        data: z.string().max(MAX_FRAME),
        tag: z.string().max(32),
      })
      .strict()
      .parse(value)
    const seq = BigInt(frame.seq)
    if (seq !== this.received) throw new Error('控制消息序号不匹配')
    const tag = Buffer.from(frame.tag, 'base64')
    if (tag.length !== 16) throw new Error('控制消息认证失败')
    const decipher = createDecipheriv('aes-256-gcm', this.rx.subarray(0, 32), this.iv(this.rx, seq))
    decipher.setAAD(Buffer.from(`screenbridge-v1:${seq}`))
    decipher.setAuthTag(tag)
    const plain = Buffer.concat([
      decipher.update(Buffer.from(frame.data, 'base64')),
      decipher.final(),
    ])
    this.received++
    return JSON.parse(plain.toString('utf8'))
  }
}

const messageSchema = z.discriminatedUnion('kind', [
  z.object({ kind: z.literal('auth') }),
  z.object({ kind: z.literal('accepted') }),
  z.object({
    kind: z.literal('request'),
    id: z.string().max(32),
    method: z.string().max(40),
    payload: z.unknown(),
  }),
  z.object({
    kind: z.literal('response'),
    id: z.string().max(32),
    result: z.unknown().optional(),
    error: z.string().max(400).optional(),
  }),
  z.object({ kind: z.literal('event'), name: z.string().max(40), payload: z.unknown() }),
])

export class SecureChannel extends EventEmitter {
  private buffer = ''
  private clientNonce = ''
  private serverNonce = ''
  private codec?: SessionCodec
  private authenticated = false
  private sequence = 0
  private pending = new Map<
    string,
    { resolve: (v: unknown) => void; reject: (e: Error) => void; timer: NodeJS.Timeout }
  >()
  private timer: NodeJS.Timeout
  private handler?: (method: string, payload: unknown) => Promise<unknown>
  constructor(
    private socket: Socket,
    private key: Buffer,
    private server: boolean,
  ) {
    super()
    socket.setNoDelay(true)
    socket.setKeepAlive(true, 5000)
    socket.setEncoding('utf8')
    this.timer = setTimeout(() => this.fail('配对超时，请检查配对码和网络'), 8000)
    socket.on('data', (chunk: string) => this.consume(chunk))
    socket.on('error', (e) => this.emit('fault', e.message))
    socket.on('close', () => {
      clearTimeout(this.timer)
      for (const entry of this.pending.values()) {
        clearTimeout(entry.timer)
        entry.reject(new Error('对端连接已断开'))
      }
      this.pending.clear()
      this.emit('closed')
    })
    if (!server) {
      this.clientNonce = randomBytes(32).toString('hex')
      this.write({ kind: 'hello', version: 1, nonce: this.clientNonce })
    }
  }
  get ready() {
    return this.authenticated && !this.socket.destroyed
  }
  get remoteAddress() {
    return this.socket.remoteAddress?.replace(/^::ffff:/, '') ?? ''
  }
  onRequest(handler: (method: string, payload: unknown) => Promise<unknown>) {
    this.handler = handler
  }
  private write(value: unknown) {
    this.socket.write(JSON.stringify(value) + '\n')
  }
  private send(value: unknown) {
    if (!this.codec || this.socket.destroyed) throw new Error('对端尚未连接')
    this.write(this.codec.seal(value))
  }
  private fail(message: string) {
    this.emit('fault', message)
    this.socket.destroy()
  }
  private consume(chunk: string) {
    this.buffer += chunk
    let end: number
    while ((end = this.buffer.indexOf('\n')) !== -1) {
      const line = this.buffer.slice(0, end)
      this.buffer = this.buffer.slice(end + 1)
      if (Buffer.byteLength(line) > MAX_FRAME) {
        this.fail('控制消息超出大小限制')
        return
      }
      try {
        this.receive(JSON.parse(line))
      } catch {
        this.fail('配对认证失败或控制消息无效')
        return
      }
      if (this.socket.destroyed) return
    }
    if (Buffer.byteLength(this.buffer) > MAX_FRAME) this.fail('控制消息超出大小限制')
  }
  private receive(raw: unknown) {
    if (!this.codec) {
      if (this.server) {
        const hello = z
          .object({ kind: z.literal('hello'), version: z.literal(1), nonce: nonceSchema })
          .strict()
          .parse(raw)
        this.clientNonce = hello.nonce
        this.serverNonce = randomBytes(32).toString('hex')
        this.write({ kind: 'challenge', nonce: this.serverNonce })
      } else {
        const challenge = z
          .object({ kind: z.literal('challenge'), nonce: nonceSchema })
          .strict()
          .parse(raw)
        this.serverNonce = challenge.nonce
      }
      this.codec = new SessionCodec(this.key, this.clientNonce, this.serverNonce, this.server)
      if (!this.server) this.send({ kind: 'auth' })
      return
    }
    const msg = messageSchema.parse(this.codec.open(raw))
    if (!this.authenticated) {
      if (msg.kind !== (this.server ? 'auth' : 'accepted')) throw new Error('握手未完成')
      this.authenticated = true
      clearTimeout(this.timer)
      if (this.server) this.send({ kind: 'accepted' })
      this.emit('ready')
      return
    }
    if (msg.kind === 'response') {
      const entry = this.pending.get(msg.id)
      if (entry) {
        clearTimeout(entry.timer)
        this.pending.delete(msg.id)
        msg.error ? entry.reject(new Error(msg.error)) : entry.resolve(msg.result)
      }
    } else if (msg.kind === 'request') {
      Promise.resolve()
        .then(() => {
          if (!this.handler) throw new Error('设备尚未就绪')
          return this.handler(msg.method, msg.payload)
        })
        .then(
          (result) => {
            if (this.ready) this.send({ kind: 'response', id: msg.id, result })
          },
          (error) => {
            if (this.ready)
              this.send({
                kind: 'response',
                id: msg.id,
                error: String(error instanceof Error ? error.message : error).slice(0, 400),
              })
          },
        )
        .catch(() => this.close())
    } else if (msg.kind === 'event') this.emit('event', msg.name, msg.payload)
    else throw new Error('重复握手消息')
  }
  request(method: string, payload: unknown = null, timeout = 10000): Promise<unknown> {
    if (!this.ready) return Promise.reject(new Error('另一台电脑尚未连接'))
    const id = String(++this.sequence)
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id)
        reject(new Error('对端响应超时'))
      }, timeout)
      this.pending.set(id, { resolve, reject, timer })
      try {
        this.send({ kind: 'request', id, method, payload })
      } catch (e) {
        clearTimeout(timer)
        this.pending.delete(id)
        reject(e)
      }
    })
  }
  notify(name: string, payload: unknown) {
    if (this.ready) this.send({ kind: 'event', name, payload })
  }
  close() {
    this.socket.destroy()
  }
}
