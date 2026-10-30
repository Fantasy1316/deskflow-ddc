import { afterEach, describe, expect, it, vi } from 'vitest'
import { createSSRApp, effectScope, h, shallowRef } from 'vue'
import { renderToString } from 'vue/server-renderer'
import { configSchema, defaultConfig } from '../src/shared/config'
import type { Action, Snapshot } from '../src/shared/types'
import { displayMappingValid, pictureCommandLabel, saveThenStart } from '../src/shared/workspace'
import { useDisplayTest } from '../src/composables/useDisplayTest'
import { useBridge } from '../src/composables/useBridge'
import Dashboard from '../src/components/Dashboard.vue'
import DisplayPanel from '../src/components/DisplayPanel.vue'
import PairingPanel from '../src/components/PairingPanel.vue'
import RuntimeError from '../src/components/RuntimeError.vue'

function snapshot(): Snapshot {
  return {
    config: {
      ...defaultConfig('MacBook'),
      displayId: 'monitor-unique-id',
      localInput: 15,
      peerInput: 17,
    },
    deviceId: 'mac',
    platform: 'darwin',
    addresses: [],
    monitors: [{ id: 'monitor-unique-id', name: 'DELL U2723', readable: false }],
    cursorDisplays: [],
    sessionLayout: null,
    running: false,
    listening: false,
    connected: false,
    engine: 'stopped',
    peer: {
      id: 'win',
      name: '工作电脑',
      platform: 'win32',
      fingerprint: '',
      inputConfigured: true,
      modifierMapping: 'original',
    },
    activeDeviceId: 'mac',
    displayDeviceId: 'win',
    displayStatus: 'commanded',
    switching: false,
    pairingCode: '',
    hasSavedPairing: false,
    dependencies: { deskflow: true, displayControl: true, accessibility: true },
    diagnostics: { directory: '', error: null },
    logs: [],
    lastError: null,
    preview: false,
  }
}
afterEach(() => {
  vi.unstubAllGlobals()
  vi.useRealTimers()
})

describe('truthful workspace state', () => {
  it('keeps picture command history independent of keyboard ownership and physical verification', async () => {
    const state = snapshot()
    expect(pictureCommandLabel(state)).toBe('最近指令切到 工作电脑')
    const html = await renderToString(
      createSSRApp({
        render: () =>
          h(Dashboard, { state, busy: false, displayConfigured: true, execute: async () => true }),
      }),
    )
    expect(html).toContain('键鼠当前归属')
    expect(html).toContain('软件未确认物理画面')
    expect(html).toContain('切到 MacBook')
    expect(html).toContain('切到 工作电脑')
    expect(html).not.toContain('一块屏幕，两台电脑。')
    expect(pictureCommandLabel({ ...state, displayStatus: 'uncertain' })).toBe('画面来源待确认')
    expect(pictureCommandLabel({ ...state, displayStatus: 'uninitialized' })).toBe(
      '尚无已记录的画面指令',
    )
  })
  it('does not count identical, missing or invalid input values as configured', () => {
    const config = snapshot().config
    expect(displayMappingValid(config)).toBe(true)
    expect(displayMappingValid({ ...config, peerInput: 15 })).toBe(false)
    expect(displayMappingValid({ ...config, localInput: null })).toBe(false)
    expect(displayMappingValid({ ...config, peerInput: 65536 })).toBe(false)
    expect(displayMappingValid({ ...config, peerInput: 1.5 })).toBe(false)
  })
  it('explains an unreadable display without declaring it unsupported', async () => {
    const html = await renderToString(
      createSSRApp({
        render: () =>
          h(DisplayPanel, { state: snapshot(), busy: false, execute: async () => true }),
      }),
    )
    expect(html).toContain('读不到不代表不支持写入')
    expect(html).toContain('切换能力需实测')
    expect(html).toContain('显示器设备标识')
    expect(html).toContain('当前页面有效')
    expect(html).not.toMatch(/<option[^>]*>[^<]*monitor-unique-id/)
  })
})

describe('display verification workflow', () => {
  it('waits for actual action success and then separate user confirmation', async () => {
    let resolve!: (value: boolean) => void
    const run = vi.fn(
      () =>
        new Promise<boolean>((r) => {
          resolve = r
        }),
    )
    const scope = effectScope()
    const test = scope.run(() => useDisplayTest(() => snapshot().config, run))!
    try {
      const promise = test.test('local')
      expect(test.results.value.local).toBe('sending')
      test.confirm('local', true)
      expect(test.results.value.local).toBe('sending')
      resolve(true)
      await promise
      expect(test.results.value.local).toBe('sent')
      test.confirm('local', true)
      expect(test.results.value.local).toBe('confirmed')
      expect(run).toHaveBeenCalledWith({ type: 'testInput', value: 15 })
    } finally {
      scope.stop()
    }
  })
  it('does not confirm failed commands and invalidates results when mapping changes', async () => {
    const config = shallowRef(snapshot().config)
    const scope = effectScope()
    const test = scope.run(() =>
      useDisplayTest(
        () => config.value,
        async () => false,
      ),
    )!
    try {
      await test.test('peer')
      expect(test.results.value.peer).toBe('failed')
      test.confirm('peer', true)
      expect(test.results.value.peer).toBe('failed')
      config.value = { ...config.value, displayId: 'another-monitor' }
      expect(test.results.value.peer).toBe('idle')
    } finally {
      scope.stop()
    }
  })
  it('discards a response for an input mapping that changed during the test', async () => {
    let resolve!: (value: boolean) => void
    const config = shallowRef(snapshot().config),
      scope = effectScope()
    const test = scope.run(() =>
      useDisplayTest(
        () => config.value,
        () =>
          new Promise((r) => {
            resolve = r
          }),
      ),
    )!
    try {
      const pending = test.test('peer')
      config.value = { ...config.value, peerInput: 18 }
      config.value = { ...config.value, peerInput: 17 }
      resolve(true)
      await pending
      expect(test.results.value.peer).toBe('idle')
    } finally {
      scope.stop()
    }
  })
})

describe('pairing actions and recovery', () => {
  it('never starts a connection after save failure, and otherwise awaits saving first', async () => {
    const run = vi.fn(async (action: Action) => action.type !== 'save')
    expect(await saveThenStart(run, snapshot().config, 'code')).toBe(false)
    expect(run.mock.calls.map(([action]) => action.type)).toEqual(['save'])
    let resolve!: (value: boolean) => void
    const actions: string[] = []
    const promise = saveThenStart(
      async (action) => {
        actions.push(action.type)
        return action.type === 'save'
          ? new Promise((r) => {
              resolve = r
            })
          : true
      },
      snapshot().config,
      'code',
    )
    expect(actions).toEqual(['save'])
    resolve(true)
    expect(await promise).toBe(true)
    expect(actions).toEqual(['save', 'start'])
  })
  it('allows a saved receiver pairing to start without a new code', async () => {
    const state = snapshot()
    state.config.role = 'client'
    state.hasSavedPairing = true
    const html = await renderToString(
      createSSRApp({
        render: () => h(PairingPanel, { state, busy: false, connect: async () => true }),
      }),
    )
    expect(html).toContain('已保存配对，留空复用')
    expect(html).toMatch(/<button[^>]*class="button primary"[^>]*>[^]*?开始共享/)
    expect(html).not.toMatch(/<button[^>]*class="button primary"[^>]*disabled[^>]*>[^<]*开始共享/)
  })
  it('shows persistent, copyable error details and recovery actions', async () => {
    const html = await renderToString(
      createSSRApp({
        render: () =>
          h(RuntimeError, {
            failure: { action: 'switch', message: 'native write failed' },
            busy: false,
            running: true,
            preview: false,
          }),
      }),
    )
    expect(html).toContain('切换画面与键鼠遇到问题')
    expect(html).toContain('copyable-text')
    expect(html).toContain('native write failed')
    expect(html).toContain('停止共享')
    expect(html).toContain('导出诊断日志')
    expect(html).toContain('关闭错误提示')
  })
  it('returns false for hardware actions in preview and retains errors beyond toast timeout', async () => {
    vi.useFakeTimers()
    vi.stubGlobal('window', {})
    let bridge!: ReturnType<typeof useBridge>
    await renderToString(
      createSSRApp({
        setup() {
          bridge = useBridge()
          return () => h('div')
        },
      }),
    )
    expect(await bridge.act({ type: 'testInput', value: 17 })).toBe(false)
    await vi.advanceTimersByTimeAsync(7100)
    expect(bridge.notice.value).toBe('')
    expect(bridge.failure.value?.action).toBe('testInput')
    bridge.dismissFailure()
    expect(bridge.failure.value).toBeNull()
  })
})

it('keeps the bridge locked for the entire save/connect sequence', async () => {
  vi.useFakeTimers()
  let releaseSave!: () => void
  const calls: string[] = []
  vi.stubGlobal('window', {
    screenbridge: {
      act: async (action: Action) => {
        calls.push(action.type)
        if (action.type === 'save')
          await new Promise<void>((resolve) => {
            releaseSave = resolve
          })
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
  const connection = bridge.saveAndStart(snapshot().config)
  expect(bridge.busy.value).toBe(true)
  expect(await bridge.act({ type: 'scan' })).toBe(false)
  expect(calls).toEqual(['save'])
  releaseSave()
  expect(await connection).toBe(true)
  expect(calls).toEqual(['save', 'start'])
  expect(bridge.busy.value).toBe(false)
  expect(bridge.pendingAction.value).toBeNull()
})

describe('standalone display mode', () => {
  it('keeps legacy profiles in sharing mode and rejects unknown modes', () => {
    const legacy = { ...snapshot().config } as Record<string, unknown>
    delete legacy.mode
    expect(configSchema.parse(legacy).mode).toBe('sharing')
    expect(configSchema.safeParse({ ...legacy, mode: 'display-only' }).success).toBe(true)
    expect(configSchema.safeParse({ ...legacy, mode: 'unknown' }).success).toBe(false)
  })
  it('enables display buttons without a peer or engine and hides keyboard transfer language', async () => {
    const state = snapshot()
    state.config.mode = 'display-only'
    state.config.role = 'client'
    state.peer = null
    state.running = true
    state.displayDeviceId = null
    state.displayTarget = 'peer'
    const html = await renderToString(
      createSSRApp({
        render: () =>
          h(Dashboard, { state, busy: false, displayConfigured: true, execute: async () => true }),
      }),
    )
    expect(html).toContain('切到本机输入')
    expect(html).toContain('切到另一输入')
    expect(html).toContain('无需运行对端应用')
    expect(html).toContain('不建立网络连接，不启动 Deskflow')
    expect(html).not.toContain('等待连接 · 接收键鼠')
    expect(html).not.toContain('配对设备')
    expect(html).not.toContain('Ctrl + Alt + Esc')
    expect(html).not.toMatch(/<button[^>]*disabled[^>]*>[^<]*切到本机输入/)
    expect(pictureCommandLabel(state)).toBe('最近指令切到另一输入（17）')
    state.displayStatus = 'uncertain'
    expect(pictureCommandLabel(state)).toBe('画面来源待确认')
  })
  it('offers activation only after valid mapping and keeps configuration available offline', async () => {
    const state = snapshot()
    state.config.mode = 'display-only'
    state.config.displayId = ''
    state.peer = null
    const html = await renderToString(
      createSSRApp({
        render: () =>
          h(Dashboard, { state, busy: false, displayConfigured: false, execute: async () => true }),
      }),
    )
    expect(html).toMatch(/<button[^>]*disabled[^>]*>\s*启用显示器切换/)
    expect(html).toContain('保存两个输入源，测试往返切换')
  })
})
