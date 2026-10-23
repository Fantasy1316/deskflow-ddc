import { spawn, type ChildProcess } from 'node:child_process'
import { stopOwnedProcess } from '../core/process-stop.js'

type Command = 'list' | 'set' | 'host' | 'peer' | 'pointer'
interface Pending {
  id: number
  resolve(value: string): void
  reject(error: Error): void
  timer: NodeJS.Timeout
}

/** Private stdio transport. No network listener, shell interpolation or arbitrary commands. */
export class WindowsHelper {
  private child?: ChildProcess
  private pending?: Pending
  private sequence = 0
  private queue: Promise<unknown> = Promise.resolve()
  private closing?: Promise<void>
  private buffer = ''
  private errors = ''
  private generation = 0
  private blocked = false
  constructor(private script: string) {}
  request(
    action: Command,
    details: { monitorId?: string; value?: number; x?: number; y?: number } = {},
  ) {
    const command = { ...details, action }
    const generation = this.generation
    const result = this.queue.then(() => {
      if (generation !== this.generation) throw new Error('Windows 原生助手会话已结束')
      return this.send(command, generation)
    })
    this.queue = result.catch(() => {})
    return result
  }
  private async send(command: object, generation: number): Promise<string> {
    if (this.closing) await this.closing
    if (generation !== this.generation) throw new Error('Windows 原生助手会话已结束')
    if (this.child && this.blocked)
      throw new Error('Windows 原生助手尚未退出，已暂停后续命令；请导出日志并检查后台进程')
    if (!this.child) this.launch()
    const child = this.child!
    return new Promise((resolve, reject) => {
      const id = ++this.sequence
      const timer = setTimeout(() => {
        if (this.pending?.id !== id) return
        const error = new Error('Windows 原生助手响应超时，已终止本次助手；请重试或导出日志')
        this.fail(error)
        void this.close().catch(() => {})
      }, 10000)
      this.pending = { id, resolve, reject, timer }
      child.stdin!.write(JSON.stringify({ ...command, id }) + '\n', (error) => {
        if (error && this.pending?.id === id) {
          this.fail(error)
          void this.close().catch(() => {})
        }
      })
    })
  }
  private launch() {
    const child = spawn(
      'powershell.exe',
      [
        '-NoLogo',
        '-NoProfile',
        '-NonInteractive',
        '-ExecutionPolicy',
        'Bypass',
        '-File',
        this.script,
        '-Action',
        'serve',
      ],
      { windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] },
    )
    this.child = child
    this.blocked = false
    this.buffer = ''
    this.errors = ''
    child.stdout!.setEncoding('utf8')
    child.stdout!.on('data', (chunk: string) => {
      if (this.child !== child) return
      this.buffer += chunk
      let end: number
      while ((end = this.buffer.indexOf('\n')) >= 0) {
        const line = this.buffer.slice(0, end)
        this.buffer = this.buffer.slice(end + 1)
        try {
          if (Buffer.byteLength(line) > 1024 * 1024) throw new Error('助手响应过大')
          const response = JSON.parse(line),
            pending = this.pending
          if (!pending || response.id !== pending.id || typeof response.ok !== 'boolean')
            throw new Error('助手响应格式或序号无效')
          clearTimeout(pending.timer)
          this.pending = undefined
          response.ok
            ? pending.resolve(JSON.stringify(response.result))
            : pending.reject(new Error(String(response.error).slice(0, 700)))
        } catch (error) {
          this.fail(error instanceof Error ? error : new Error(String(error)))
          void this.close().catch(() => {})
          return
        }
      }
      if (Buffer.byteLength(this.buffer) > 1024 * 1024) {
        this.fail(new Error('助手响应过大'))
        void this.close().catch(() => {})
      }
    })
    child.stderr!.on('data', (chunk) => {
      if (this.child === child) this.errors = (this.errors + chunk.toString()).slice(-4096)
    })
    child.stdin!.on('error', (error) => {
      if (this.child === child) {
        this.fail(error)
        void this.close().catch(() => {})
      }
    })
    child.once('error', (error) => {
      if (this.child === child) {
        this.fail(error)
        void this.close().catch(() => {})
      }
    })
    child.once('close', () => {
      if (this.child !== child) return
      this.child = undefined
      this.fail(new Error(this.errors.trim().slice(0, 700) || 'Windows 原生助手已退出'))
    })
  }
  private fail(error: Error) {
    if (!this.pending) return
    clearTimeout(this.pending.timer)
    this.pending.reject(error)
    this.pending = undefined
  }
  close(): Promise<void> {
    ++this.generation
    if (this.closing) return this.closing
    this.fail(new Error('Windows 原生助手已停止'))
    const child = this.child
    if (!child) return Promise.resolve()
    this.blocked = true
    const closing = stopOwnedProcess(child).finally(() => {
      if (this.child === child && (child.exitCode != null || child.signalCode != null))
        this.child = undefined
      if (this.closing === closing) this.closing = undefined
    })
    this.closing = closing
    return closing
  }
}
