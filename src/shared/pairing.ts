import type { Snapshot } from './types'

export function canStartSharing(
  state: Pick<Snapshot, 'config' | 'hasSavedPairing' | 'pairingCode'>,
  enteredCode: string,
): boolean {
  if (state.config.role === 'host') return true
  const code = enteredCode.trim()
  const reuse = state.hasSavedPairing && (!code || code === state.pairingCode)
  const address = !!state.config.peerAddress.trim() || !!state.config.fallbackIps?.length
  return (reuse || !!code) && (reuse || address)
}
