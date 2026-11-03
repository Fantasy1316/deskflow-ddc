export type TransferStatus =
  | 'pending'
  | 'waiting'
  | 'connecting'
  | 'transferring'
  | 'conflict'
  | 'paused'
  | 'complete'
  | 'cancelled'
export interface FileTransfer {
  id: string
  label: string
  direction: 'send' | 'receive'
  status: TransferStatus
  peerName: string
  entries: number
  totalBytes: number
  completedBytes: number
  currentFile: string
  destination: string | null
  error: string | null
  createdAt: string
  conflict?: {
    token: string
    name: string
    directory: boolean
    existingDirectory: boolean
  } | null
}
export type FileAction =
  | { type: 'sendFiles' }
  | { type: 'sendDropped'; token: string }
  | { type: 'accept' | 'resume' | 'pause' | 'cancel' | 'remove' | 'open'; id: string }
  | {
      type: 'resolveConflict'
      id: string
      token: string
      decision: 'replace' | 'keepBoth' | 'stop'
    }
export const transferLabels: Record<TransferStatus, string> = {
  pending: '等待接收确认',
  waiting: '等待对端接收或继续',
  connecting: '正在连接',
  transferring: '传输与校验中',
  conflict: '等待处理同名项目',
  paused: '已暂停',
  complete: '已完成',
  cancelled: '已取消',
}
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  const units = ['KiB', 'MiB', 'GiB', 'TiB', 'PiB']
  let value = bytes / 1024,
    index = 0
  while (value >= 1024 && index < units.length - 1) {
    value /= 1024
    index++
  }
  return `${value.toFixed(value >= 100 ? 0 : 1)} ${units[index]}`
}
export function transferProgress(job: FileTransfer): number {
  if (job.status === 'complete') return 100
  return job.totalBytes ? Math.min(99, Math.floor((job.completedBytes / job.totalBytes) * 100)) : 0
}
export function transferActive(job: FileTransfer): boolean {
  return ['waiting', 'connecting', 'transferring', 'conflict'].includes(job.status)
}

export interface FileTransferNetwork {
  controlRttMs: number
  baselineRttMs: number
  sendLimitBytesPerSecond: number | null
}
