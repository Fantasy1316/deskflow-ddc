import { afterEach, describe, expect, it, vi } from 'vitest'
import { createSSRApp, h, nextTick } from 'vue'
import { renderToString } from 'vue/server-renderer'
import { useBridge } from '../src/composables/useBridge'
import { defaultConfig } from '../src/shared/config'
import { actionSchema } from '../src/shared/actions'
import { outputChoiceLabel, outputUnavailableReason } from '../src/shared/output-selection'
import type { Action, OutputDisplay, Snapshot } from '../src/shared/types'
import OutputRecovery from '../src/components/OutputRecovery.vue'
import DisplayMethodSettings from '../src/components/DisplayMethodSettings.vue'
import RuntimeError from '../src/components/RuntimeError.vue'

function state(): Snapshot {
  return {
    config: { ...defaultConfig('Mac'), displayMethod: 'signal', outputId: 'external' },
    platform: 'darwin',
    deviceId: 'local',
    peer: { id: 'peer', name: 'Windows' },
    connected: true,
    running: true,
    switching: false,
    engine: 'ready',
    displayStatus: 'commanded',
    outputSwitch: { token: 'aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee', target: 'peer' },
  } as Snapshot
}
const screen: OutputDisplay = {
  id: 'external',
  name: '外接屏',
  internal: false,
  bounds: { x: 0, y: 0, width: 1920, height: 1080 },
}
const html = async (component: Parameters<typeof h>[0], props: Record<string, unknown>) => {
  const context: { teleports?: Record<string, string> } = {}
  const result = await renderToString(
    createSSRApp(() => h(component, props)),
    context,
  )
  return result + (context.teleports?.body ?? '')
}
afterEach(() => {
  vi.unstubAllGlobals()
  vi.useRealTimers()
})

describe('switch feedback and recovery', () => {
  it('shows the settling stage in both modes and keeps emergency recovery enabled', async () => {
    for (const method of ['ddc', 'signal'] as const) {
      const snapshot = state()
      snapshot.config.displayMethod = method
      snapshot.switching = true
      snapshot.switchProgress = { stage: 'waiting-display' }
      const result = await html(OutputRecovery, { state: snapshot, busy: true })
      expect(result).toContain('等待显示器稳定…')
      expect(result).toContain('aria-current="step"')
      expect(result).toContain('停止共享并恢复输出')
      expect(result).not.toContain('disabled')
    }
  })
  it('replaces the success row with real stage progress and keeps emergency stop available while busy', async () => {
    const snapshot = state()
    snapshot.switching = true
    snapshot.switchProgress = { stage: 'disabling-source' }
    const result = await html(OutputRecovery, { state: snapshot, busy: true })
    expect(result).toContain('停用原输出…')
    expect(result).toContain('恢复目标输出')
    expect(result).toContain('转移键鼠')
    expect(result).toContain('aria-current="step"')
    expect(result).not.toContain('最近切换至')
    expect(result).toContain('停止共享并恢复输出')
    expect(result).not.toContain('disabled')
    snapshot.emergencyStopping = true
    const stopping = await html(OutputRecovery, { state: snapshot, busy: true })
    expect(stopping).toContain('正在停止共享并恢复输出')
    expect(stopping).toContain('disabled')
  })

  it('shows success as a compact row, with details collapsed; restoration failures expand', async () => {
    const snapshot = state()
    const success = await html(OutputRecovery, { state: snapshot, busy: false })
    expect(success).toContain('output-recovery')
    expect(success).not.toContain('class="panel')
    expect(success).toContain('最近切换至 Windows')
    expect(success).toContain('重试切换')
    expect(success).toContain('恢复输出')
    expect(success).toContain('<details')
    expect(success).not.toMatch(/<details[^>]*\bopen\b/)
    snapshot.displayStatus = 'uncertain'
    const failure = await html(OutputRecovery, { state: snapshot, busy: false })
    expect(failure).toContain('output-failure')
    expect(failure).toContain('输出恢复尚未完成')
  })

  it('puts the failure reason above collapsed details and offers the original target retry', async () => {
    const result = await html(RuntimeError, {
      failure: {
        action: 'switch',
        message: '恢复目标输出失败：Windows 原生调用超时',
        retry: { type: 'switch', target: 'local' },
      },
      busy: true,
      running: true,
      preview: false,
    })
    expect(result.indexOf('恢复目标输出失败')).toBeLessThan(result.indexOf('<details'))
    expect(result).toContain('重试此操作')
    expect(result).toContain('检查显示器配置')
    // The queued action can be busy, while emergency recovery remains enabled.
    expect(result).not.toMatch(/<button[^>]*disabled[^>]*>\s*停止共享并恢复输出/)
  })
})

describe('one-save output selection', () => {
  it('marks internal, retained, mirrored, inactive and driver-rejected outputs as unavailable', () => {
    const snapshot = state()
    snapshot.config.retainedDisplayId = 'internal'
    expect(outputUnavailableReason(snapshot, screen)).toBeNull()
    expect(outputChoiceLabel(snapshot, screen)).toContain('已配置共用屏')
    for (const change of [
      { internal: true },
      { cursorId: 'internal' },
      { mirrored: true },
      { active: false },
      { unavailableReason: '仅支持一块活动输出' },
    ]) {
      expect(outputUnavailableReason(snapshot, { ...screen, ...change })).toBeTruthy()
      expect(outputChoiceLabel(snapshot, { ...screen, ...change })).toContain('不可选')
    }
  })

  it('allows scanning the draft method before saving and explains why outputs cannot be selected', async () => {
    const snapshot = state()
    snapshot.config.displayMethod = 'ddc'
    snapshot.outputDisplays = [
      screen,
      { ...screen, id: 'built-in', name: '内置屏幕', internal: true },
    ]
    const result = await html(DisplayMethodSettings, {
      state: snapshot,
      locked: false,
      dirty: true,
      method: 'signal',
      output: 'external',
    })
    expect(result).toContain('无需先保存')
    expect(result).not.toContain('先保存切屏方式')
    expect(result).toMatch(/<option[^>]*value="built-in"[^>]*disabled/)
    expect(result).toContain('内置屏不能作为允许断开的共用输出')
    expect(actionSchema.parse({ type: 'scanDisplays', method: 'signal' }).type).toBe('scanDisplays')
    expect(actionSchema.safeParse({ type: 'scanDisplays', method: 'shell' }).success).toBe(false)
    expect(actionSchema.safeParse({ type: 'emergencyStop', command: 'kill-all' }).success).toBe(
      false,
    )
  })
})

it('emergency stop bypasses an in-flight action and suppresses its late cancellation error', async () => {
  vi.useFakeTimers()
  let rejectSwitch!: (error: Error) => void, releaseRecovery!: () => void
  const calls: string[] = []
  vi.stubGlobal('window', {
    screenbridge: {
      act: (action: Action) => {
        calls.push(action.type)
        if (action.type === 'switch')
          return new Promise<void>((_, reject) => {
            rejectSwitch = reject
          })
        if (action.type === 'emergencyStop')
          return new Promise<void>((resolve) => {
            releaseRecovery = resolve
          })
        return Promise.resolve()
      },
    },
  })
  let bridge!: ReturnType<typeof useBridge>
  await renderToString(
    createSSRApp({
      setup() {
        bridge = useBridge()
        return () => h('div')
      },
    }),
  )
  const switching = bridge.act({ type: 'switch', target: 'peer' })
  expect(bridge.busy.value).toBe(true)
  const recovery = bridge.act({ type: 'emergencyStop' })
  expect(calls).toEqual(['switch', 'emergencyStop'])
  expect(bridge.emergencyBusy.value).toBe(true)
  expect(await bridge.act({ type: 'emergencyStop' })).toBe(false)
  expect(await bridge.act({ type: 'scan' })).toBe(false)
  rejectSwitch(new Error('会话已结束'))
  expect(await switching).toBe(false)
  expect(bridge.failure.value).toBeNull()
  expect(bridge.busy.value).toBe(true)
  expect(bridge.pendingAction.value).toBe('emergencyStop')
  releaseRecovery()
  expect(await recovery).toBe(true)
  expect(bridge.busy.value).toBe(false)
  expect(bridge.failure.value).toBeNull()
  expect(bridge.notice.value).toContain('共享已停止')
})

it('offers a stopped local-output retry without requiring a connection or engine', async () => {
  vi.useFakeTimers()
  const calls: Action[] = []
  vi.stubGlobal('window', {
    screenbridge: {
      act: async (action: Action) => {
        calls.push(action)
        if (calls.length === 1) throw new Error('恢复显示输出失败：temporary failure')
      },
    },
  })
  let bridge!: ReturnType<typeof useBridge>
  await renderToString(
    createSSRApp({
      setup() {
        bridge = useBridge()
        return () => h('div')
      },
    }),
  )
  bridge.state.value = {
    ...state(),
    running: false,
    connected: false,
    engine: 'stopped',
    peer: null,
    outputSwitch: null,
    localOutputRecoveryPending: true,
  }
  expect(await bridge.act({ type: 'stop' })).toBe(false)
  expect(bridge.failure.value?.retry).toEqual({ type: 'stop' })
  const error = await html(RuntimeError, {
    failure: bridge.failure.value,
    running: false,
    busy: false,
    preview: false,
  })
  expect(error).toContain('重试此操作')
  expect(error).toContain('恢复本机显示输出')
  expect(await bridge.act(bridge.failure.value!.retry!)).toBe(true)
  expect(bridge.failure.value).toBeNull()
  expect(calls).toEqual([{ type: 'stop' }, { type: 'stop' }])
})

it('clears the retried error on success while preserving unrelated runtime diagnostics', async () => {
  vi.useFakeTimers()
  for (const unrelated of [false, true]) {
    let calls = 0
    vi.stubGlobal('window', {
      screenbridge: {
        act: async () => {
          if (++calls === 1) throw new Error('发送输入源指令失败')
        },
      },
    })
    let bridge!: ReturnType<typeof useBridge>
    await renderToString(
      createSSRApp({
        setup() {
          bridge = useBridge()
          return () => h('div')
        },
      }),
    )
    const action: Action = { type: 'switch', target: 'local' }
    expect(await bridge.act(action)).toBe(false)
    bridge.state.value = {
      ...bridge.state.value,
      lastError: unrelated ? '另一项运行错误' : '发送输入源指令失败',
    }
    await nextTick()
    expect(await bridge.act(action)).toBe(true)
    if (unrelated) expect(bridge.failure.value?.message).toBe('另一项运行错误')
    else expect(bridge.failure.value).toBeNull()
  }
})
