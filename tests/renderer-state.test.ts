import { describe, expect, it, vi } from 'vitest'
import { effectScope, nextTick, shallowRef } from 'vue'
import { useConfigDraft } from '../src/composables/useConfigDraft'
import { initializeBridge } from '../src/composables/bridge-state'
import { defaults } from '../electron/core/config'
import type { Snapshot } from '../src/shared/types'

describe('renderer state races', () => {
  it('merges unrelated config updates without replacing edited fields, then accepts a save', async () => {
    const source = shallowRef(defaults('original')),
      scope = effectScope()
    const editor = scope.run(() => useConfigDraft(() => source.value))!
    try {
      editor.draft.shortcut = 'control+alt+F9'
      source.value = { ...source.value, deskflowPath: '/selected/core' }
      await nextTick()
      expect(editor.draft.shortcut).toBe('control+alt+F9')
      expect(editor.draft.deskflowPath).toBe('/selected/core')
      source.value = editor.value()
      await nextTick()
      source.value = { ...source.value, shortcut: 'control+alt+F10' }
      await nextTick()
      expect(editor.draft.shortcut).toBe('control+alt+F10')
    } finally {
      scope.stop()
    }
  })
  it('ignores a slow bootstrap snapshot after a push, and all updates after disposal', async () => {
    let push!: (value: Snapshot) => void, resolve!: (value: Snapshot) => void
    const apply = vi.fn(),
      unsubscribe = vi.fn()
    const connection = initializeBridge(
      {
        subscribe: (callback) => {
          push = callback
          return unsubscribe
        },
        snapshot: () =>
          new Promise((r) => {
            resolve = r
          }),
      },
      apply,
    )
    const latest = { running: true } as Snapshot,
      stale = { running: false } as Snapshot
    push(latest)
    resolve(stale)
    await connection.ready
    expect(apply).toHaveBeenCalledExactlyOnceWith(latest)
    connection.dispose()
    push(stale)
    expect(apply).toHaveBeenCalledTimes(1)
    expect(unsubscribe).toHaveBeenCalledOnce()
  })
})
