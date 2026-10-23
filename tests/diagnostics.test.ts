import { afterEach, describe, expect, it } from 'vitest'
import {
  appendFileSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { DiagnosticLog, diagnosticSnapshot } from '../electron/core/diagnostics'
import { defaults } from '../electron/core/config'
import type { Snapshot } from '../src/shared/types'
import { engineLifecycleLine, safeEngineLine } from '../electron/core/engine-log'

const roots: string[] = []
function fixture() {
  const root = mkdtempSync(join(tmpdir(), 'screenbridge-log-'))
  roots.push(root)
  return root
}
afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true })
})
describe('persistent diagnostic journals and exports', () => {
  it('preserves previous launches and exception stacks, and deduplicates the memory fallback', () => {
    const root = fixture(),
      dir = join(root, 'logs'),
      file = join(root, 'report.json')
    const first = new DiagnosticLog(dir)
    first.record('info', 'app.start', { version: 'test', role: 'host' })
    const error = new Error('Cannot read properties of null', {
      cause: new Error('identity not saved'),
    })
    first.record('error', 'engine.failed', error)
    const restarted = new DiagnosticLog(dir)
    restarted.record('info', 'app.start', { version: 'new' })
    restarted.exportTo(file, { port: 24800 })
    const report = JSON.parse(readFileSync(file, 'utf8'))
    expect(report.records).toHaveLength(3)
    expect(new Set(report.records.map((entry: { runId: string }) => entry.runId)).size).toBe(2)
    expect(report.records[1].details.stack).toContain('diagnostics.test.ts')
    expect(report.records[1].details.cause.message).toBe('identity not saved')
    expect(report.context.port).toBe(24800)
  })
  it('removes pairing secrets, private keys and content fields from both disk and exports', () => {
    const root = fixture(),
      dir = join(root, 'logs'),
      file = join(root, 'report.json')
    const logger = new DiagnosticLog(dir)
    const code = 'SB1-' + 'q'.repeat(43)
    logger.protect(code)
    const cyclic: Record<string, unknown> = {
      pairingCode: code,
      clipboardContent: 'sensitive-clipboard',
      private_key: 'private-material',
      secret: 'stored-secret',
    }
    cyclic.self = cyclic
    logger.record('error', 'failure', {
      cyclic,
      error: new Error(
        `request ${code.slice(4)} at /Users/alice/work/app.ts and C:\\Users\\bob\\AppData\\app.js`,
      ),
      pem: '-----BEGIN RSA PRIVATE KEY-----\nprivate-body\n-----END RSA PRIVATE KEY-----',
      message: '-----BEGIN PRIVATE KEY-----\nunclosed-key-material',
    })
    logger.exportTo(file, { pairingCode: code, clipboardEnabled: true, clipboardMegabytes: 4 })
    for (const text of [
      readFileSync(join(dir, 'screenbridge.jsonl'), 'utf8'),
      readFileSync(file, 'utf8'),
    ]) {
      for (const secret of [
        code,
        code.slice(4),
        'sensitive-clipboard',
        'private-material',
        'stored-secret',
        'private-body',
        'unclosed-key-material',
        'alice',
        'bob',
      ])
        expect(text).not.toContain(secret)
      expect(text).toContain('[REDACTED')
    }
    expect(JSON.parse(readFileSync(file, 'utf8')).context.clipboardEnabled).toBe(true)
  })
  it('rotates files within the configured disk budget and retains the newest event', () => {
    const dir = join(fixture(), 'logs'),
      logger = new DiagnosticLog(dir, { maxFileBytes: 1024, maxFiles: 3 })
    for (let index = 0; index < 80; index++)
      logger.record('info', 'test.event', { index, message: 'rotation' })
    const files = readdirSync(dir)
    expect(files).toHaveLength(3)
    expect(files.every((file) => statSync(join(dir, file)).size <= 1024)).toBe(true)
    expect(readFileSync(join(dir, 'screenbridge.jsonl'), 'utf8')).toContain('"index":79')
    logger.record('error', 'large', { message: 'x'.repeat(10000) })
    expect(statSync(join(dir, 'screenbridge.jsonl')).size).toBeLessThanOrEqual(1024)
    expect(readFileSync(join(dir, 'screenbridge.jsonl'), 'utf8')).toContain('详细内容已省略')
  })
  it('survives an interrupted final line and reports skipped corruption', () => {
    const root = fixture(),
      dir = join(root, 'logs'),
      file = join(root, 'report.json')
    new DiagnosticLog(dir).record('info', 'before.crash')
    appendFileSync(join(dir, 'screenbridge.jsonl'), '{"partial":')
    const restarted = new DiagnosticLog(dir)
    restarted.record('info', 'after.restart')
    restarted.exportTo(file, {})
    const report = JSON.parse(readFileSync(file, 'utf8'))
    expect(report.skippedLines).toBe(1)
    expect(report.records.map((entry: { event: string }) => entry.event)).toEqual([
      'before.crash',
      'after.restart',
    ])
  })
  it('keeps the app running on a write failure and exports the in-memory records', () => {
    const root = fixture(),
      blocked = join(root, 'not-a-directory'),
      file = join(root, 'report.json')
    writeFileSync(blocked, 'occupied')
    const logger = new DiagnosticLog(blocked)
    expect(() =>
      logger.record('error', 'test.diskFailure', new Error('still reportable')),
    ).not.toThrow()
    expect(logger.status.error).toBeTruthy()
    logger.exportTo(file, {})
    expect(JSON.parse(readFileSync(file, 'utf8')).records[0].details.message).toBe(
      'still reportable',
    )
    expect(() => logger.exportTo(join(blocked, 'report.json'), {})).toThrow()
  })
  it('exports only approved state fields without copying surrounding secret files', () => {
    const root = fixture(),
      dir = join(root, 'logs'),
      file = join(root, 'report.json')
    writeFileSync(join(root, 'settings.json'), 'DO-NOT-INCLUDE-SETTINGS')
    writeFileSync(join(root, 'identity.pem'), 'DO-NOT-INCLUDE-IDENTITY')
    const logger = new DiagnosticLog(dir)
    const state = {
      config: defaults('Test Mac'),
      pairingCode: 'NEVER-INCLUDE-PAIRING',
      deviceId: 'test-device',
      logs: [],
      diagnostics: logger.status,
      extraSecret: 'NEVER-INCLUDE-EXTRA',
    } as unknown as Snapshot
    logger.record('info', 'app.start')
    logger.exportTo(file, diagnosticSnapshot(state))
    const text = readFileSync(file, 'utf8')
    for (const secret of ['DO-NOT-INCLUDE', 'NEVER-INCLUDE']) expect(text).not.toContain(secret)
    expect(JSON.parse(text).context.config.controlPort).toBe(49728)
  })
})
describe('Deskflow output privacy', () => {
  it('keeps useful lifecycle info and excludes verbose input payloads', () => {
    expect(engineLifecycleLine('INFO: connected to server')).toBe(true)
    expect(engineLifecycleLine('INFO: switch from "sb-a" to "sb-b"')).toBe(true)
    for (const line of [
      'DEBUG: failed sending clipboard contents',
      'INFO: clipboard text=secret',
      'DEBUG1: key down abc',
    ]) {
      expect(engineLifecycleLine(line)).toBe(false)
    }
    expect(safeEngineLine('DEBUG: cannot send clipboard contents: abc')).toBeUndefined()
  })
  it('redacts an upstream error that quotes a malformed clipboard header', () => {
    expect(safeEngineLine('ERROR: clipboard invalid size header: PRIVATE-CONTENT')).toBe(
      'ERROR: clipboard invalid size header: [REDACTED]',
    )
    expect(safeEngineLine('ERROR: cannot read server.conf: missing )')).toContain('missing )')
  })
})
