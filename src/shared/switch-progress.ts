import type { SwitchStage } from './types'
export const stageLabels: Record<SwitchStage, string> = {
  'restoring-target': '恢复目标输出',
  'transferring-input': '转移键鼠',
  'disabling-source': '停用原输出',
  committing: '保留输出状态',
  'writing-input': '发送输入源指令',
  'waiting-display': '等待显示器稳定',
  'restoring-outputs': '恢复显示输出',
}
export function progressSteps(stage: SwitchStage, signal: boolean): SwitchStage[] {
  if (stage === 'restoring-outputs') return ['restoring-outputs']
  return signal
    ? [
        'restoring-target',
        'transferring-input',
        'disabling-source',
        'committing',
        'waiting-display',
      ]
    : ['transferring-input', 'writing-input', 'waiting-display']
}
