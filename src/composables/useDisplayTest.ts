import { computed, ref, watch } from 'vue'
import type { Config } from '../shared/types'
import type { ActionRunner } from '../shared/workspace'
import { mappingSignature } from '../shared/ddc'
export type TestStatus = 'idle' | 'sending' | 'sent' | 'confirmed' | 'mismatch' | 'failed'
export type InputTarget = 'local' | 'peer'
export function useDisplayTest(config: () => Config, execute: ActionRunner) {
  const signature = computed(() => mappingSignature(config()))
  let generation = 0
  const results = ref<Record<InputTarget, TestStatus>>({ local: 'idle', peer: 'idle' })
  const remembered = ref(false)
  const returnedLocal = ref(false)
  watch(
    signature,
    () => {
      generation++
      results.value = { local: 'idle', peer: 'idle' }
      remembered.value = false
      returnedLocal.value = false
    },
    { flush: 'sync' },
  )
  async function test(target: InputTarget) {
    const value = target === 'local' ? config().localInput : config().peerInput
    if (value === null || results.value[target] === 'sending') return
    const before = generation
    results.value[target] = 'sending'
    remembered.value = false
    returnedLocal.value = false
    const accepted = await execute({ type: 'testInput', value })
    if (before === generation) results.value[target] = accepted ? 'sent' : 'failed'
  }
  function confirm(target: InputTarget, correct: boolean) {
    if (results.value[target] !== 'sent') return
    results.value[target] = correct ? 'confirmed' : 'mismatch'
    if (target === 'local') returnedLocal.value = correct && results.value.peer === 'confirmed'
  }
  const canRemember = computed(
    () =>
      returnedLocal.value &&
      results.value.local === 'confirmed' &&
      results.value.peer === 'confirmed',
  )
  async function remember() {
    if (!canRemember.value) return
    const before = generation
    const accepted = await execute({ type: 'rememberDdcProfile' })
    if (before === generation) remembered.value = accepted
  }
  return { results, test, confirm, canRemember, remembered, remember }
}
