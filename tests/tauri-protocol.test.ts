import { expect, it } from 'vitest'
import { SessionCodec } from '../electron/core/channel'
import fixture from './fixtures/control-protocol.json'
it('the existing TypeScript engine protocol matches the Rust interoperability fixture', () => {
  const sender = new SessionCodec(
    Buffer.from(fixture.key, 'hex'),
    fixture.clientNonce,
    fixture.serverNonce,
    true,
  )
  const receiver = new SessionCodec(
    Buffer.from(fixture.key, 'hex'),
    fixture.clientNonce,
    fixture.serverNonce,
    false,
  )
  expect(sender.seal(fixture.payload)).toEqual(fixture.frame)
  expect(receiver.open(fixture.rustFrame)).toEqual(fixture.payload)
})
