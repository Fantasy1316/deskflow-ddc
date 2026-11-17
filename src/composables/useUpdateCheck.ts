import { onScopeDispose, readonly, shallowRef } from 'vue'
import { checkForUpdates, type UpdateResult } from '../transport/updates'

type UpdateState =
  | { status: 'idle' | 'checking' }
  | { status: 'checked'; result: UpdateResult }
  | { status: 'error'; message: string }

export function useUpdateCheck(currentVersion: string) {
  const state = shallowRef<UpdateState>({ status: 'idle' })
  let pending: AbortController | undefined
  let disposed = false

  async function check() {
    if (disposed || state.value.status === 'checking') return
    const request = new AbortController()
    pending = request
    state.value = { status: 'checking' }
    try {
      const result = await checkForUpdates(currentVersion, request.signal)
      if (!request.signal.aborted) state.value = { status: 'checked', result }
    } catch (error) {
      if (!request.signal.aborted) {
        state.value = {
          status: 'error',
          message: error instanceof Error ? error.message : '检查更新失败，请稍后重试。',
        }
      }
    } finally {
      if (pending === request) pending = undefined
    }
  }

  onScopeDispose(() => {
    disposed = true
    pending?.abort()
  })
  return { state: readonly(state), check }
}
