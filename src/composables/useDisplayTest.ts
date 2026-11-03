import { computed, ref, watch } from 'vue'
import type { Config } from '../shared/types'
import type { ActionRunner } from '../shared/workspace'
export type TestStatus = 'idle' | 'sending' | 'sent' | 'confirmed' | 'mismatch' | 'failed'
export type InputTarget = 'local' | 'peer'
export function useDisplayTest(config: () => Config, execute: ActionRunner) {
  const signature = computed(() =>
    JSON.stringify([config().displayId, config().localInput, config().peerInput]),
  )
  let generation = 0
  const results = ref<Record<InputTarget, TestStatus>>({ local: 'idle', peer: 'idle' })
  watch(
    signature,
    () => {
      generation++
      results.value = { local: 'idle', peer: 'idle' }
    },
    { flush: 'sync' },
  )
  async function test(target: InputTarget) {
    const value = target === 'local' ? config().localInput : config().peerInput
    if (value === null || results.value[target] === 'sending') return
    const before = generation
    results.value[target] = 'sending'
    const accepted = await execute({ type: 'testInput', value })
    if (before === generation) results.value[target] = accepted ? 'sent' : 'failed'
  }
  function confirm(target: InputTarget, correct: boolean) {
    if (results.value[target] === 'sent') results.value[target] = correct ? 'confirmed' : 'mismatch'
  }
  return { results, test, confirm }
}
