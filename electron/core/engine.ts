import { spawn, type ChildProcess } from 'node:child_process'
import { EventEmitter } from 'node:events'
import { access, cp, mkdir, readFile, writeFile } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import { connect, type Socket } from 'node:net'
import { X509Certificate } from 'node:crypto'
import selfsigned from 'selfsigned'
import type { Config, Peer } from '../../src/shared/types.js'
import { makeServerConfig, makeSettings, parseSwitch } from './deskflow-config.js'
import { assertNoRunningEngine, assertPortAvailable, deskflowIpcPath } from './engine-preflight.js'
import { engineLifecycleLine, safeEngineLine } from './engine-log.js'
import { engineDirectory, engineLaunch } from './engine-launch.js'
import { stopOwnedProcess } from './process-stop.js'
import { DiagnosticLines } from './diagnostic-lines.js'
import { prepareRuntimeCopy } from './runtime-cache.js'

export async function findDeskflow(custom = '') {
  const candidates = custom
    ? [custom]
    : process.platform === 'darwin'
      ? ['/Applications/Deskflow.app/Contents/MacOS/deskflow-core']
      : [
          join(process.env.ProgramFiles ?? 'C:/Program Files', 'Deskflow/deskflow-core.exe'),
          join(process.env.LOCALAPPDATA ?? '', 'Programs/Deskflow/deskflow-core.exe'),
        ]
  for (const file of candidates) {
    try {
      await access(file)
      return file
    } catch {
      /* next */
    }
  }
  return ''
}
export class DeskflowEngine extends EventEmitter {
  private child?: ChildProcess
  private ipc?: Socket
  private ipcReady = false
  private retry?: NodeJS.Timeout
  private watchdog?: NodeJS.Timeout
  private stopping = false
  private failed = false
  private stopTask?: Promise<void>
  private expectedPeerFingerprint = ''
  private ownedRoot: string
  constructor(private dataDir: string) {
    super()
    this.ownedRoot = join(dataDir, 'deskflow')
  }
  async identity(): Promise<{ fingerprint: string }> {
    const path = join(this.dataDir, 'identity.pem')
    let pem: string
    try {
      pem = await readFile(path, 'utf8')
    } catch (e) {
      if ((e as NodeJS.ErrnoException).code !== 'ENOENT') throw e
      const generated = selfsigned.generate([{ name: 'commonName', value: 'Deskflow DDC' }], {
        keySize: 2048,
        algorithm: 'sha256',
        days: 3650,
      })
      pem = generated.private + '\n' + generated.cert
      await writeFile(path, pem, { mode: 0o600 })
    }
    return {
      fingerprint: new X509Certificate(pem).fingerprint256.replaceAll(':', '').toLowerCase(),
    }
  }
  async prepare(c: Config, localId: string, peer: Peer, serverAddress: string) {
    let binary = await findDeskflow(c.deskflowPath)
    if (!binary) throw new Error('未找到 Deskflow 1.27.x，请先安装或选择 deskflow-core 可执行文件')
    let root = this.ownedRoot
    if (process.platform === 'win32') {
      const runtime = join(this.dataDir, 'deskflow-runtime')
      // A private portable copy keeps Deskflow's existing settings and trust store untouched.
      await prepareRuntimeCopy(dirname(binary), runtime)
      binary = join(runtime, 'deskflow-core.exe')
      root = join(runtime, 'settings')
    }
    await mkdir(join(root, 'tls'), { recursive: true })
    await cp(join(this.dataDir, 'identity.pem'), join(root, 'tls/identity.pem'))
    const { text, trust } = makeSettings(c, localId, peer.id, root, peer.fingerprint, serverAddress)
    await writeFile(join(root, 'Deskflow.conf'), text, { mode: 0o600 })
    const host = c.role === 'host' ? localId : peer.id,
      client = c.role === 'client' ? localId : peer.id
    await writeFile(
      join(root, 'server.conf'),
      makeServerConfig(
        host,
        client,
        c.shortcut,
        process.platform,
        c.role === 'host' ? c.layoutSide : 'off',
        c.crossScreenVideo,
      ),
    )
    await writeFile(
      join(root, 'tls', c.role === 'host' ? 'trusted-clients' : 'trusted-servers'),
      trust,
    )
    return { binary, settings: join(root, 'Deskflow.conf') }
  }
  async assertAvailable(c: Config) {
    if (this.child) throw new Error('键鼠共享进程已经运行')
    await assertNoRunningEngine()
    const directory = engineDirectory(this.dataDir)
    if (directory) await assertNoRunningEngine(deskflowIpcPath(directory))
    if (c.role === 'host') await assertPortAvailable(c.deskflowPort)
  }
  private fail(message: string) {
    if (this.failed || this.stopping) return
    this.failed = true
    clearTimeout(this.watchdog)
    clearTimeout(this.retry)
    this.emit('state', 'error')
    this.emit('diagnostic', message)
    // Only stop our child. The global IPC endpoint can belong to another process.
    void this.terminate().catch((error) =>
      this.emit('diagnostic', '无法停止故障引擎，已阻止重复启动', error),
    )
  }
  async start(c: Config, localId: string, peer: Peer, serverAddress: string) {
    await this.assertAvailable(c)
    const prepared = await this.prepare(c, localId, peer, serverAddress)
    this.emit('trace', 'engine.prepared', {
      binary: prepared.binary,
      settings: prepared.settings,
      role: c.role,
      localId,
      peerId: peer.id,
      peerFingerprint: peer.fingerprint,
      serverAddress,
      port: c.deskflowPort,
    })
    this.stopping = false
    this.failed = false
    this.expectedPeerFingerprint = peer.fingerprint
    const launch = await engineLaunch(this.dataDir)
    const child = spawn(
      prepared.binary,
      [c.role === 'host' ? 'server' : 'client', '--settings', prepared.settings],
      {
        windowsHide: true,
        stdio: ['ignore', 'pipe', 'pipe'],
        env: {
          ...launch.env,
          XDG_CONFIG_HOME: join(this.dataDir, 'isolated-config'),
          XDG_STATE_HOME: join(this.dataDir, 'isolated-state'),
        },
      },
    )
    this.child = child
    this.emit('state', 'starting')
    this.emit('trace', 'engine.spawn', {
      pid: child.pid,
      role: c.role,
      ipcPath: launch.ipcPath,
      temporaryDirectory: launch.directory,
    })
    child.once('close', () => {
      if (this.child === child) {
        clearTimeout(this.retry)
        clearTimeout(this.watchdog)
        this.ipc?.destroy()
        this.ipc = undefined
        this.ipcReady = false
        this.child = undefined
        this.emit('state', this.stopping ? 'stopped' : 'error')
      }
    })
    let outputSeen = false,
      ipcReady = false,
      connectionState = ''
    const diagnostics = new DiagnosticLines((line) => this.emit('diagnostic', line))
    child.once('close', () => diagnostics.flush())
    const buffers = new Map<string, string>()
    const consume = (key: string, chunk: Buffer) => {
      if (chunk.length) outputSeen = true
      const text = (buffers.get(key) ?? '') + chunk.toString('utf8')
      const lines = text.split(/\r?\n/)
      buffers.set(key, (lines.pop() ?? '').slice(-16384))
      for (const raw of lines) {
        const line = safeEngineLine(raw)
        if (!line) continue
        if (engineLifecycleLine(line)) this.emit('trace', 'engine.output', { line })
        // The first hello discovers the installed build ID, then connectIpc retries it.
        if (/core ipc client version mismatch \(client: screenbridge-probe, server:/.test(line))
          continue
        if (
          /fingerprint does not match|failed to verify (server|client) certificate fingerprint/i.test(
            line,
          )
        ) {
          this.fail(
            '键鼠引擎证书校验失败：当前连接的证书与 Deskflow DDC 配对身份不一致。请停止两端共享，完整退出独立 Deskflow 及其后台服务，再重新启动；不要关闭证书校验。',
          )
          continue
        }
        if (/an instance of deskflow core is already running/i.test(line)) {
          // Qt reports all shared-memory creation failures as a duplicate instance,
          // including ENOSPC. Preserve the actual output without asserting a port conflict.
          this.emit('trace', 'engine.startupRejected', { reason: 'single-instance', line })
          this.fail(
            'Deskflow 单实例资源初始化失败：可能已有引擎运行，或系统共享内存资源不足。请导出诊断日志以确认原因。',
          )
          continue
        }
        if (/cannot listen for clients/i.test(line)) {
          this.emit('trace', 'engine.startupRejected', { reason: 'listen', line })
          this.fail(
            '已有 Deskflow 引擎或程序占用了键鼠端口。请退出独立 Deskflow 及后台服务后重试，Deskflow DDC 未接管该进程。',
          )
          continue
        }
        const change = parseSwitch(line)
        if (change && c.role === 'host') this.emit('switch', change)
        // Never forward raw debug key or clipboard payloads into application logs.
        if (/failed to register hotkey/i.test(line)) {
          diagnostics.report(
            'Deskflow 未能注册快捷键。Windows 请避开系统保留的 F12；也请检查是否被其他程序占用，在共享设置中更换快捷键后重新开始共享。',
          )
        } else if (
          /\b(ERROR|FATAL|CRITICAL|WARNING|WARN)\b/.test(line) ||
          /cannot |failed |another.*running/i.test(line)
        )
          diagnostics.report(line.slice(0, 500))
      }
    }
    child.stdout?.on('data', (chunk) => consume('out', chunk))
    child.stderr?.on('data', (chunk) => consume('err', chunk))
    child.on('error', (e) => this.fail(`键鼠引擎无法启动或运行：${e.message}`))
    child.once('exit', (code, signal) => {
      if (this.child !== child) return
      for (const [key, remaining] of buffers) if (remaining) consume(key, Buffer.from('\n'))
      this.emit('trace', 'engine.exit', {
        code,
        signal,
        stopping: this.stopping,
        failed: this.failed,
      })
      clearTimeout(this.retry)
      clearTimeout(this.watchdog)
      this.ipc?.destroy()
      this.ipc = undefined
      this.ipcReady = false
      if (this.child === child) this.child = undefined
      this.emit('state', this.failed ? 'error' : this.stopping ? 'stopped' : 'error')
      if (!this.stopping && !this.failed)
        this.emit(
          'diagnostic',
          code === 4
            ? 'Deskflow 配置加载失败（4）。请查看前面的配置文件和行号错误，并确认两端均使用最新版 Deskflow DDC。'
            : `Deskflow 已退出（${code ?? signal}）。请检查辅助功能权限、版本，以及是否另有 Deskflow 实例正在运行。`,
        )
    })
    this.watchdog = setTimeout(() => {
      this.emit('trace', 'engine.startupTimeout', {
        pid: child.pid,
        role: c.role,
        outputSeen,
        ipcReady,
        connectionState,
        ipcPath: launch.ipcPath,
      })
      if (!ipcReady)
        this.fail(
          '键鼠引擎启动超时，尚未建立本机 IPC 通道，已停止本次引擎。请导出诊断日志；此时尚不能归因于对端网络或防火墙。',
        )
      else
        this.emit(
          'diagnostic',
          c.role === 'host'
            ? `本机引擎已启动，仍在等待另一台电脑连接键鼠端口 ${c.deskflowPort}。请同时查看对端诊断日志。`
            : `本机引擎已启动，但尚未连上 ${serverAddress}:${c.deskflowPort}，请核对提供端引擎日志及该端口的网络连通性。`,
        )
    }, 18000)
    this.connectIpc(0, launch.ipcPath, (state) => {
      if (state.startsWith('hello=')) ipcReady = true
      else connectionState = state
    })
  }
  private connectIpc(attempt: number, path: string, observe: (state: string) => void) {
    if (!this.child || this.stopping || this.failed || attempt >= 72) return
    const child = this.child
    const socket = connect(path)
    this.ipc = socket
    this.ipcReady = false
    let buffer = ''
    let helloDone = false
    socket.setEncoding('utf8')
    socket.on('connect', () => {
      if (this.child !== child || this.stopping || this.failed) {
        socket.destroy()
        return
      }
      socket.write('hello=screenbridge-probe\n')
    })
    socket.on('data', (chunk: string) => {
      if (this.child !== child || this.stopping || this.failed) return
      buffer += chunk
      let end: number
      while ((end = buffer.indexOf('\n')) >= 0) {
        const line = buffer.slice(0, end)
        buffer = buffer.slice(end + 1)
        if (Buffer.byteLength(line) > 65536) {
          socket.destroy()
          return
        }
        if (line.startsWith('hello=') || line.startsWith('connectionState=')) observe(line)
        if (line.startsWith('hello=')) this.ipcReady = true
        if (/^(versionMismatch|hello|connectionState|peerFingerprint)=/.test(line))
          this.emit('trace', 'engine.ipc', { line })
        if (this.failed || this.stopping) return
        if (line.startsWith('versionMismatch=') && !helloDone) {
          helloDone = true
          socket.write(`hello=${line.slice('versionMismatch='.length)}\n`)
        } else if (line.startsWith('peerFingerprint=')) {
          const actual = line.slice('peerFingerprint='.length).replaceAll(':', '').toLowerCase()
          if (actual !== this.expectedPeerFingerprint)
            this.fail(
              '键鼠引擎连接到了另一份证书，未通过配对身份检查。请完整退出独立 Deskflow 后重新启动共享。',
            )
        } else if (line === 'connectionState=Connected') {
          clearTimeout(this.watchdog)
          this.emit('state', 'ready')
        } else if (/^connectionState=(Disconnected|Connecting|Listening)$/.test(line))
          this.emit('state', 'starting')
      }
      if (Buffer.byteLength(buffer) > 65536) socket.destroy()
    })
    socket.on('error', () => {
      /* bounded retry below */
    })
    socket.once('close', () => {
      if (this.ipc === socket) {
        this.ipc = undefined
        this.ipcReady = false
      }
      if (!this.stopping && !this.failed && this.child === child)
        this.retry = setTimeout(() => this.connectIpc(attempt + 1, path, observe), 250)
    })
  }
  private terminate(): Promise<void> {
    if (this.stopTask) return this.stopTask
    const child = this.child
    if (!child) return Promise.resolve()
    const socket = this.ipc
    const requestStop =
      this.ipcReady && socket && !socket.destroyed
        ? () => {
            socket.write('stop\n')
            this.emit('trace', 'engine.stopRequested', { pid: child.pid, via: 'ipc' })
          }
        : undefined
    const task = stopOwnedProcess(child, 1500, 1500, requestStop).finally(() => {
      socket?.destroy()
      if (this.stopTask === task) this.stopTask = undefined
    })
    this.stopTask = task
    return task
  }
  async stop() {
    this.stopping = true
    clearTimeout(this.retry)
    clearTimeout(this.watchdog)
    await this.terminate()
  }
}
