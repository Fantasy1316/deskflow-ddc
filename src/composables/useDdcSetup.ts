import { computed } from 'vue'
import type { Config, InputProtocol, Snapshot } from '../shared/types'
import { effectiveProtocol, inputPresets, savedDdcProfile } from '../shared/ddc'

export function useDdcSetup(state: () => Snapshot, draft: Config) {
  const protocol = computed(() => effectiveProtocol(draft))
  const selected = computed(() => state().monitors.find((m) => m.id === draft.displayId))
  const saved = computed(() => savedDdcProfile(state(), selected.value))
  function protocolChanged(previous: InputProtocol, next: InputProtocol) {
    if (next === previous) return
    const translate = (value: number | null) => {
      const port = inputPresets(previous).find((p) => p.value === value)?.port
      return inputPresets(next).find((p) => p.port === port)?.value ?? null
    }
    // Carry interface choices across protocols, never reinterpret raw numbers.
    draft.localInput = translate(draft.localInput)
    draft.peerInput = translate(draft.peerInput)
  }
  function restoreSaved() {
    const profile = saved.value
    if (!profile) return false
    draft.ddcProtocol = 'auto'
    draft.ddcAutoProtocol = profile.protocol
    draft.localInput = profile.localInput
    draft.peerInput = profile.peerInput
    return true
  }
  function selectDisplay() {
    if (draft.displayId === state().config.displayId) {
      const c = state().config
      Object.assign(draft, {
        ddcProtocol: c.ddcProtocol,
        ddcAutoProtocol: c.ddcAutoProtocol,
        localInput: c.localInput,
        peerInput: c.peerInput,
      })
      return
    }
    if (!restoreSaved()) {
      draft.ddcProtocol = 'auto'
      draft.ddcAutoProtocol = 'standard'
      draft.localInput = null
      draft.peerInput = null
    }
  }
  function applyRecommendation(_protocol: InputProtocol) {
    const detection = state().ddcDetection
    if (detection?.displayId === draft.displayId && detection.savedProfile) restoreSaved()
  }
  return { protocol, saved, restoreSaved, applyRecommendation, protocolChanged, selectDisplay }
}
