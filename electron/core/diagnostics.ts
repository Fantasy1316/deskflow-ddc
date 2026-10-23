import {
  appendFileSync,
  closeSync,
  existsSync,
  mkdirSync,
  openSync,
  readFileSync,
  readSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs'
import { join } from 'node:path'
import { randomUUID } from 'node:crypto'
import type { Config, Snapshot } from '../../src/shared/types.js'

type Level = 'info' | 'success' | 'error'
interface RecordEntry {
  time: string
  runId: string
  sequence: number
  level: Level
  event: string
  details: unknown
}
interface Options {
  maxFileBytes?: number
  maxFiles?: number
}

/** Deliberately never accepts a whole config, protocol frame, environment or identity PEM. */
export function diagnosticConfig(c: Config) {
  return {
    deviceName: c.deviceName,
    role: c.role,
    peerAddress: c.peerAddress,
    controlPort: c.controlPort,
    deskflowPort: c.deskflowPort,
    deskflowPath: c.deskflowPath,
    displayId: c.displayId,
    localInput: c.localInput,
    peerInput: c.peerInput,
    shortcut: c.shortcut,
    clipboardEnabled: c.clipboard,
    clipboardMegabytes: c.clipboardMegabytes,
    layoutSide: c.layoutSide,
    crossScreenVideo: c.crossScreenVideo,
    retainedDisplayId: c.retainedDisplayId,
  }
}
export function diagnosticSnapshot(s: Snapshot) {
  return {
    config: diagnosticConfig(s.config),
    deviceId: s.deviceId,
    platform: s.platform,
    addresses: s.addresses,
    monitors: s.monitors,
    cursorDisplays: s.cursorDisplays,
    sessionLayout: s.sessionLayout,
    running: s.running,
    listening: s.listening,
    connected: s.connected,
    engine: s.engine,
    peer: s.peer,
    activeDeviceId: s.activeDeviceId,
    displayDeviceId: s.displayDeviceId,
    displayStatus: s.displayStatus,
    lastDisplayDeviceId: s.lastDisplayDeviceId,
    switching: s.switching,
    dependencies: s.dependencies,
    lastError: s.lastError,
    recentActivity: s.logs,
    diagnostics: s.diagnostics,
  }
}

/** Synchronous, bounded writes keep startup/crash errors durable without a pending flush queue. */
export class DiagnosticLog {
  readonly runId = randomUUID()
  private sequence = 0
  private secrets = new Set<string>()
  private memory: RecordEntry[] = []
  private opened = false
  private storageError: string | null = null
  private maxFileBytes: number
  private maxFiles: number
  constructor(
    readonly directory: string,
    options: Options = {},
  ) {
    this.maxFileBytes = options.maxFileBytes ?? 2 * 1024 * 1024
    this.maxFiles = options.maxFiles ?? 5
    if (this.maxFileBytes < 1024 || this.maxFiles < 1 || this.maxFiles > 20)
      throw new Error('日志保留配置无效')
  }
  get status() {
    return { directory: this.directory, error: this.storageError }
  }
  protect(secret: string) {
    if (secret.length < 16) return
    this.secrets.add(secret)
    if (secret.startsWith('SB1-')) this.secrets.add(secret.slice(4))
  }
  text(value: string) {
    let safe = value
      .replace(
        /-----BEGIN [^-]*(?:PRIVATE KEY|CERTIFICATE)-----[\s\S]*?(?:-----END [^-]+-----|$)/g,
        '[REDACTED PEM]',
      )
      .replace(/SB1-[A-Za-z0-9_-]{43}/g, '[REDACTED PAIRING CODE]')
    for (const secret of this.secrets) safe = safe.split(secret).join('[REDACTED]')
    safe = safe
      .replace(/\/(Users|home)\/[^/\s]+/g, '/$1/<user>')
      .replace(/([A-Z]:[\\/]Users[\\/])[^\\/\s]+/gi, '$1<user>')
    return safe.slice(0, 8192)
  }
  sanitize(value: unknown): unknown {
    const seen = new WeakSet<object>()
    let nodes = 0
    const visit = (item: unknown, depth = 0): unknown => {
      if (++nodes > 256 || depth > 8) return '[TRUNCATED]'
      if (typeof item === 'string') return this.text(item)
      if (item === null || typeof item === 'boolean' || typeof item === 'number') return item
      if (typeof item !== 'object') return item === undefined ? null : `[${typeof item}]`
      if (seen.has(item)) return '[CIRCULAR]'
      seen.add(item)
      if (item instanceof Error) {
        const error = item as Error & { code?: unknown }
        return visit(
          {
            name: error.name,
            message: error.message,
            stack: error.stack,
            code: error.code,
            cause: error.cause,
          },
          depth + 1,
        )
      }
      if (Array.isArray(item)) return item.slice(0, 80).map((child) => visit(child, depth + 1))
      return Object.fromEntries(
        Object.entries(item)
          .slice(0, 80)
          .map(([key, child]) => {
            const normalized = key.replace(/[^a-z]/gi, '').toLowerCase()
            const sensitive =
              /pairingcode|password|passphrase|privatekey|secret/.test(normalized) ||
              [
                'pem',
                'key',
                'clipboardcontent',
                'clipboarddata',
                'clipboardtext',
                'keyboardcontent',
                'keyevent',
                'mouseevent',
              ].includes(normalized)
            return [this.text(key), sensitive ? '[REDACTED]' : visit(child, depth + 1)]
          }),
      )
    }
    try {
      return visit(value)
    } catch {
      return '[UNAVAILABLE]'
    }
  }
  private path(index = 0) {
    return join(this.directory, index ? `screenbridge.${index}.jsonl` : 'screenbridge.jsonl')
  }
  private open() {
    if (this.opened) return
    mkdirSync(this.directory, { recursive: true, mode: 0o700 })
    const file = this.path()
    if (existsSync(file) && statSync(file).size) {
      const fd = openSync(file, 'r')
      try {
        const tail = Buffer.alloc(1)
        readSync(fd, tail, 0, 1, statSync(file).size - 1)
        if (tail[0] !== 10) appendFileSync(file, '\n', { mode: 0o600 })
      } finally {
        closeSync(fd)
      }
    }
    this.opened = true
  }
  record(level: Level, event: string, details?: unknown) {
    const entry: RecordEntry = {
      time: new Date().toISOString(),
      runId: this.runId,
      sequence: ++this.sequence,
      level,
      event: this.text(event).slice(0, 200),
      details: this.sanitize(details),
    }
    let line = JSON.stringify(entry) + '\n'
    if (Buffer.byteLength(line) > Math.min(this.maxFileBytes, 64 * 1024)) {
      entry.details = { omitted: '日志条目过大，详细内容已省略' }
      line = JSON.stringify(entry) + '\n'
    }
    this.memory.push(entry)
    if (this.memory.length > 200) this.memory.shift()
    try {
      this.open()
      if (
        existsSync(this.path()) &&
        statSync(this.path()).size + Buffer.byteLength(line) > this.maxFileBytes
      ) {
        rmSync(this.path(this.maxFiles - 1), { force: true })
        for (let i = this.maxFiles - 2; i >= 0; i--)
          if (existsSync(this.path(i))) renameSync(this.path(i), this.path(i + 1))
      }
      appendFileSync(this.path(), line, { encoding: 'utf8', mode: 0o600 })
      this.storageError = null
    } catch (error) {
      this.storageError = this.text(error instanceof Error ? error.message : String(error))
    }
  }
  exportTo(file: string, context: unknown) {
    const records = new Map<string, unknown>()
    let skippedLines = 0
    for (let i = this.maxFiles - 1; i >= 0; i--) {
      try {
        if (!existsSync(this.path(i))) continue
        // Only read our bounded journal files, never the surrounding settings or TLS files.
        if (statSync(this.path(i)).size > this.maxFileBytes + 64 * 1024) {
          skippedLines++
          continue
        }
        for (const line of readFileSync(this.path(i), 'utf8').split('\n')) {
          if (!line.trim()) continue
          try {
            const entry = JSON.parse(line) as RecordEntry
            records.set(`${entry.runId}:${entry.sequence}`, this.sanitize(entry))
          } catch {
            skippedLines++
          }
        }
      } catch (error) {
        this.storageError = this.text(error instanceof Error ? error.message : String(error))
      }
    }
    for (const entry of this.memory)
      records.set(`${entry.runId}:${entry.sequence}`, this.sanitize(entry))
    const report = {
      format: 'screenbridge-diagnostics',
      formatVersion: 1,
      exportedAt: new Date().toISOString(),
      runId: this.runId,
      storageError: this.storageError,
      skippedLines,
      context: this.sanitize(context),
      records: [...records.values()],
    }
    writeFileSync(file, JSON.stringify(report, null, 2) + '\n', { encoding: 'utf8', mode: 0o600 })
  }
}
