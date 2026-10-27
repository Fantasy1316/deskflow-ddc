export type Role = 'host' | 'client'
export type ScreenSide = 'left' | 'right' | 'top' | 'bottom'
export interface Config {
  deviceName: string
  role: Role
  peerAddress: string
  fallbackIps?: string[]
  controlPort: number
  deskflowPort: number
  deskflowPath: string
  displayId: string
  localInput: number | null
  peerInput: number | null
  shortcut: string
  clipboard: boolean
  clipboardMegabytes: number
  layoutSide: 'off' | ScreenSide
  crossScreenVideo: 'manual' | 'follow' | 'keep'
  retainedDisplayId: string
}
export interface CursorDisplay {
  id: string
  name: string
  internal: boolean
  bounds: { x: number; y: number; width: number; height: number }
}
export interface SessionLayout {
  hostSide: Config['layoutSide']
  video: Config['crossScreenVideo']
}
export interface Monitor {
  id: string
  name: string
  currentInput?: number
  readable: boolean
}
export interface Peer {
  id: string
  name: string
  platform: string
  fingerprint: string
  inputConfigured: boolean
  retainedDisplay?: boolean
}
export interface LogEntry {
  id: number
  time: string
  level: 'info' | 'success' | 'error'
  message: string
}
export interface DiscoveredDevice {
  id: string
  name: string
  hostname: string
  addresses: string[]
  port: number
}
export interface Snapshot {
  revision?: number
  hostname?: string
  engineSource?: 'bundled' | 'external'
  discoveredDevices?: DiscoveredDevice[]
  discovering?: boolean
  connection?: { address: string; source: string; authenticated: boolean } | null
  cursorDisplays: CursorDisplay[]
  sessionLayout: SessionLayout | null
  config: Config
  deviceId: string
  platform: string
  addresses: string[]
  monitors: Monitor[]
  running: boolean
  listening: boolean
  connected: boolean
  engine: 'stopped' | 'starting' | 'ready' | 'error'
  peer: Peer | null
  activeDeviceId: string | null
  displayDeviceId: string | null
  switching: boolean
  pairingCode: string
  displayStatus?: 'uninitialized' | 'commanded' | 'uncertain'
  lastDisplayDeviceId?: string | null
  dependencies: { deskflow: boolean; displayControl: boolean; accessibility: boolean }
  diagnostics: { directory: string; error: string | null }
  logs: LogEntry[]
  lastError: string | null
  preview: boolean
}
export type { Action } from './actions.js'
import type { Action } from './actions.js'
export interface RendererError {
  source: 'window.error' | 'window.unhandledrejection' | 'vue'
  message: string
  stack?: string
}
export interface BridgeAPI {
  snapshot(): Promise<Snapshot>
  act(action: Action): Promise<void>
  reportError(error: RendererError): Promise<void>
  subscribe(listener: (state: Snapshot) => void): () => void
}
declare global {
  interface Window {
    screenbridge?: BridgeAPI
  }
}
