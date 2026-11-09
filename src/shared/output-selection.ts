import type { OutputDisplay, Snapshot } from './types'
export function outputUnavailableReason(state: Snapshot, screen: OutputDisplay): string | null {
  if (screen.internal) return '内置屏不能作为允许断开的共用输出'
  if ((screen.cursorId || screen.id) === state.config.retainedDisplayId)
    return '此屏幕已配置为独立屏幕，请选择另一块共用屏幕'
  if (screen.mirrored) return '镜像输出暂不支持，请先改为扩展显示'
  if (screen.active === false) return '输出未启用，请先恢复视频连接'
  return screen.unavailableReason || null
}
export function outputChoiceLabel(state: Snapshot, screen: OutputDisplay): string {
  const reason = outputUnavailableReason(state, screen)
  return `${screen.name} · ${screen.bounds.width} × ${screen.bounds.height} · ${screen.internal ? '内置屏' : '外接屏'}${screen.id === state.config.outputId ? ' · 已配置共用屏' : ''}${reason ? ' · 不可选：' + reason : ''}`
}
