import { reactive, watch } from 'vue'
import type { Config } from '../shared/types'
export function useConfigDraft(source: () => Config) {
  const draft = reactive<Config>({ ...source() })
  let baseline = { ...source() }
  watch(
    () => ({ ...source() }),
    (incoming) => {
      // Keep edited fields; absorb unrelated external changes such as selecting
      // the engine executable. A successful save becomes the new baseline.
      for (const key of Object.keys(incoming) as (keyof Config)[]) {
        if (JSON.stringify(draft[key]) === JSON.stringify(baseline[key]))
          Object.assign(draft, { [key]: incoming[key] })
      }
      baseline = { ...incoming }
    },
  )
  function value(): Config {
    return { ...draft, ...(draft.fallbackIps ? { fallbackIps: [...draft.fallbackIps] } : {}) }
  }
  return { draft, value }
}
