import { describe, expect, it } from 'vitest'
import { defaultConfig } from '../src/shared/config'
import { canStartSharing } from '../src/shared/pairing'

function receiver(hasSavedPairing = false) {
  return {
    config: { ...defaultConfig('receiver'), role: 'client' as const },
    hasSavedPairing,
    pairingCode: 'saved-code',
  }
}

describe('pairing start eligibility', () => {
  it('requires both an address and an entered code for the first pairing', () => {
    const state = receiver()
    expect(canStartSharing(state, '')).toBe(false)
    expect(canStartSharing(state, 'new-code')).toBe(false)
    state.config.peerAddress = 'provider.local'
    expect(canStartSharing(state, '   ')).toBe(false)
    expect(canStartSharing(state, 'new-code')).toBe(true)
    state.config.peerAddress = ''
    state.config.fallbackIps = ['192.168.1.8']
    expect(canStartSharing(state, 'new-code')).toBe(true)
  })
  it('reuses saved pairing and pinned discovery after a stop or app restart', () => {
    const state = receiver(true)
    // Session peer/address may be absent; persisted trust is the deciding state.
    expect(canStartSharing(state, '')).toBe(true)
    expect(canStartSharing(state, '   ')).toBe(true)
    expect(canStartSharing(state, ' saved-code ')).toBe(true)
  })
  it('does not use the previous peer address cache for a replacement pairing', () => {
    const state = receiver(true)
    expect(canStartSharing(state, 'replacement-code')).toBe(false)
    state.config.peerAddress = 'new-provider.local'
    expect(canStartSharing(state, 'replacement-code')).toBe(true)
    state.hasSavedPairing = false
    expect(canStartSharing(state, '')).toBe(false)
  })
  it('allows the keyboard provider to wait without entering a peer code', () => {
    const state = { ...receiver(), config: defaultConfig('provider') }
    expect(canStartSharing(state, '')).toBe(true)
  })
})
