import { createDecipheriv, hkdfSync } from 'node:crypto'
import { expect, it } from 'vitest'
import fixture from './fixtures/control-protocol.json'

// An independent Node crypto oracle checks both fixture encodings. The actual
// Rust codec checks this same fixture in protocol.rs; no legacy backend is used.
it('authenticates the protocol fixture using HKDF-SHA256 and AES-256-GCM', () => {
  const material = Buffer.from(
    hkdfSync(
      'sha256',
      Buffer.from(fixture.key, 'hex'),
      Buffer.from(fixture.clientNonce + fixture.serverNonce, 'hex'),
      'screenbridge-v1/server-to-client',
      36,
    ),
  )
  for (const frame of [fixture.frame, fixture.rustFrame]) {
    const iv = Buffer.alloc(12)
    material.copy(iv, 0, 32)
    iv.writeBigUInt64BE(BigInt(frame.seq), 4)
    const decipher = createDecipheriv('aes-256-gcm', material.subarray(0, 32), iv)
    decipher.setAAD(Buffer.from(`screenbridge-v1:${frame.seq}`))
    decipher.setAuthTag(Buffer.from(frame.tag, 'base64'))
    const plaintext = Buffer.concat([
      decipher.update(Buffer.from(frame.data, 'base64')),
      decipher.final(),
    ])
    expect(JSON.parse(plaintext.toString('utf8'))).toEqual(fixture.payload)
  }
})
