import type { Action, Config, Snapshot } from './types'

export function platformLabel(platform: string): string {
  return (
    ({ darwin: 'macOS', win32: 'Windows', linux: 'Ubuntu / Linux' } as Record<string, string>)[
      platform
    ] ?? '浏览器预览'
  )
}

export type ActionRunner = (action: Action) => Promise<boolean>
export interface ActionFailure {
  action: Action['type'] | 'snapshot' | 'runtime'
  message: string
  retry?: Action
}
export const actionLabels: Partial<Record<ActionFailure['action'], string>> = {
  saveStartup: '保存启动设置',
  checkStartup: '检测登录启动项',
  save: '保存设置',
  start: '建立共享连接',
  switch: '切换画面与键鼠',
  confirmDisplaySource: '确认当前画面来源',
  detectDisplaySource: '检测当前输入源',
  testInput: '测试显示器输入',
  checkOutput: '检测显示输出',
  restoreOutput: '恢复显示输出',
  emergencyStop: '停止共享并恢复输出',
  scanDisplays: '扫描显示器',
  scan: '扫描显示器',
  snapshot: '读取设备状态',
  discover: '发现局域网设备',
  checkAccessibility: '检测辅助功能权限',
}
export function deviceName(state: Snapshot, id: string | null | undefined): string {
  if (!id) return '未知设备'
  if (id === state.deviceId) return state.config.deviceName
  if (id === state.peer?.id) return state.peer.name
  return '另一台电脑（当前离线）'
}
export function pictureCommandLabel(state: Snapshot): string {
  const source = state.displaySource
  if (source?.kind === 'checking') return '正在识别当前输入源…'
  if (
    state.displayStatus === 'commanded' &&
    (source?.kind === 'ddc' || source?.kind === 'manual') &&
    source.device === state.displayDeviceId
  )
    return `当前来源：${deviceName(state, source.device)}（${source.kind === 'ddc' ? 'DDC 读取' : '用户确认'}）`
  if (source?.kind === 'unknown') return '请选择当前画面来源'
  if (
    state.config.displayMethod === 'signal' &&
    state.displayDeviceId &&
    state.displayStatus === 'commanded'
  )
    return `最近切换至 ${deviceName(state, state.displayDeviceId)}`
  if (state.displayStatus === 'uncertain') return '画面来源待确认'
  if (state.config.mode === 'display-only') {
    if (state.displayStatus !== 'commanded' || !state.displayTarget) return '尚无已记录的画面指令'
    return state.displayTarget === 'local'
      ? `最近指令切到 ${state.config.deviceName} 的输入`
      : `最近指令切到另一输入（${state.config.peerInput}）`
  }
  if (state.displayStatus !== 'commanded' || !state.displayDeviceId) return '尚无已记录的画面指令'
  return `最近指令切到 ${deviceName(state, state.displayDeviceId)}`
}
export function displayMappingValid(config: Config): boolean {
  if (config.displayMethod === 'signal')
    return (
      config.mode === 'sharing' && !!config.outputId.trim() && config.crossScreenVideo === 'manual'
    )
  return (
    !!config.displayId &&
    config.localInput !== null &&
    config.peerInput !== null &&
    config.localInput !== config.peerInput &&
    [config.localInput, config.peerInput].every((n) => Number.isInteger(n) && n >= 0 && n <= 65535)
  )
}
// Keep save and start ordered, including when the native save rejects.
export async function saveThenStart(
  run: ActionRunner,
  config: Config,
  pairingCode?: string,
): Promise<boolean> {
  if (!(await run({ type: 'save', config }))) return false
  return run({ type: 'start', pairingCode })
}
