import { createSSRApp, h } from 'vue'
import { renderToString } from 'vue/server-renderer'
import { describe, expect, it } from 'vitest'
import DisplayMethodSettings from '../src/components/DisplayMethodSettings.vue'
import OutputRecovery from '../src/components/OutputRecovery.vue'
import WorkspaceOverview from '../src/components/dashboard/WorkspaceOverview.vue'
import { configSchema, defaultConfig } from '../src/shared/config'
import { actionSchema } from '../src/shared/actions'
import { displayMappingValid, pictureCommandLabel } from '../src/shared/workspace'
import type { Snapshot } from '../src/shared/types'

const token = 'aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee'
function state(): Snapshot {
  return {
    config: { ...defaultConfig('本机'), displayMethod: 'signal', outputId: 'external' },
    platform: 'darwin',
    deviceId: 'local',
    peer: { id: 'peer', name: '目标电脑' },
    outputDisplays: [
      {
        id: 'external',
        name: '共用屏幕',
        internal: false,
        bounds: { x: 0, y: 0, width: 1920, height: 1080 },
      },
    ],
    displayDeviceId: null,
    displayStatus: 'uninitialized',
  } as Snapshot
}

describe('experimental output fallback', () => {
  it('preserves DDC defaults and requires manual network sharing for signal fallback', () => {
    const config = defaultConfig('desktop')
    const { displayMethod: _method, outputId: _output, ...legacy } = config
    expect(configSchema.parse(legacy).displayMethod).toBe('ddc')
    expect(configSchema.parse(legacy).outputId).toBe('')
    const signal = state().config
    expect(configSchema.safeParse(signal).success).toBe(true)
    expect(configSchema.safeParse({ ...signal, localInput: 0, peerInput: 0 }).success).toBe(true)
    expect(displayMappingValid(signal)).toBe(true)
    for (const change of [
      { mode: 'display-only' },
      { crossScreenVideo: 'follow' },
      { crossScreenVideo: 'keep' },
    ])
      expect(configSchema.safeParse({ ...signal, ...change }).success).toBe(false)
    expect(displayMappingValid({ ...signal, outputId: '' })).toBe(false)
  })

  it('explains supported display layouts without exposing Windows wake implementation details', async () => {
    for (const platform of ['darwin', 'win32'] as const) {
      const snapshot = { ...state(), platform }
      const html = await renderToString(
        createSSRApp(() =>
          h(DisplayMethodSettings, {
            state: snapshot,
            locked: false,
            dirty: false,
            method: 'signal',
            output: 'external',
          }),
        ),
      )
      expect(html).toContain('实验性')
      expect(html).toContain('无需手动确认')
      expect(html).not.toContain('秒确认')
      expect(html).toContain('检测输出控制条件')
      expect(html).toContain(
        platform === 'darwin' ? '无需安装其他软件' : '多屏或镜像模式请使用 DDC',
      )
      expect(html).not.toContain('BetterDisplay Pro')
      if (platform === 'win32') {
        expect(html).not.toContain('零位移鼠标事件')
        expect(html).toContain('提前唤醒')
      } else {
        expect(html).toContain('另一块活动屏幕')
      }
      expect(html).not.toContain('已确认支持切屏')
    }
  })

  it('shows automatic completion with retry and recovery, without blocking later switches', async () => {
    const snapshot = state()
    snapshot.connected = true
    snapshot.engine = 'ready'
    snapshot.outputSwitch = { token, target: 'peer' }
    snapshot.displayDeviceId = 'peer'
    snapshot.displayStatus = 'commanded'
    expect(pictureCommandLabel(snapshot)).toBe('最近切换至 目标电脑')
    const html = await renderToString(
      createSSRApp(() => h(OutputRecovery, { state: snapshot, busy: false })),
    )
    expect(html).toContain('最近切换至 目标电脑')
    expect(html).toContain('无需手动确认')
    expect(html).toContain('重试切换')
    expect(html).toContain('恢复输出')
    expect(html).not.toContain('disabled')
    const workspace = await renderToString(
      createSSRApp(() => h(WorkspaceOverview, { state: snapshot, busy: false })),
    )
    expect(workspace).not.toContain('disabled')
    expect(html).toContain('软件无法直接读取实际输入源')
    expect(actionSchema.parse({ type: 'restoreOutput', token }).type).toBe('restoreOutput')
    for (const invalid of [{ token: 'bad' }, { token, keep: true }, { token, output: 'other' }])
      expect(actionSchema.safeParse({ type: 'restoreOutput', ...invalid }).success).toBe(false)
    expect(actionSchema.safeParse({ type: 'confirmOutput', token, keep: true }).success).toBe(false)
    snapshot.outputSwitch = null
    expect(pictureCommandLabel(snapshot)).toBe('最近切换至 目标电脑')
  })

  it('retains recovery after a failed restoration and disables actions during switching', async () => {
    const snapshot = state()
    snapshot.connected = true
    snapshot.outputSwitch = { token, target: 'local' }
    snapshot.displayStatus = 'uncertain'
    let html = await renderToString(
      createSSRApp(() => h(OutputRecovery, { state: snapshot, busy: false })),
    )
    expect(html).toContain('输出恢复尚未完成')
    expect(html).toContain('恢复输出')
    snapshot.switching = true
    html = await renderToString(
      createSSRApp(() => h(OutputRecovery, { state: snapshot, busy: false })),
    )
    expect(html.match(/disabled/g)).toHaveLength(2)
  })

  it('keeps local recovery enabled after stopping, disconnecting or changing display settings', async () => {
    for (const displayMethod of ['signal', 'ddc'] as const) {
      const snapshot: Snapshot = {
        ...state(),
        config: { ...state().config, displayMethod },
        running: false,
        connected: false,
        engine: 'stopped',
        peer: null,
        outputSwitch: null,
        localOutputRecoveryPending: true,
      }
      let html = await renderToString(
        createSSRApp(() => h(OutputRecovery, { state: snapshot, busy: false })),
      )
      expect(html).toContain('本机显示输出尚未恢复')
      expect(html).toContain('共享已停止')
      expect(html).toContain('恢复本机显示输出')
      expect(html).not.toContain('disabled')
      expect(actionSchema.parse({ type: 'stop' })).toEqual({ type: 'stop' })
      expect(actionSchema.safeParse({ type: 'stop', outputId: 'other' }).success).toBe(false)
      html = await renderToString(
        createSSRApp(() => h(OutputRecovery, { state: snapshot, busy: true })),
      )
      expect(html).toContain('disabled')
      snapshot.localOutputRecoveryPending = false
      html = await renderToString(
        createSSRApp(() => h(OutputRecovery, { state: snapshot, busy: false })),
      )
      expect(html).not.toContain('恢复本机显示输出')
    }
  })
})
