import { createServer, connect, type Server } from 'node:net'
import { EventEmitter } from 'node:events'
import { networkInterfaces } from 'node:os'
import { z } from 'zod'
import type { Config, Peer, Snapshot } from '../../src/shared/types.js'
import { Store } from './store.js'
import { configSchema, requireReady, requireSupportedShortcut } from './config.js'
import { SecureChannel, decodePairingCode, newPairingCode } from './channel.js'
import { DeskflowEngine, findDeskflow } from './engine.js'
import { DisplayController } from '../platform/display.js'
import { SwitchFlow, type Transition } from './switch-flow.js'
import { DiagnosticLog, diagnosticConfig } from './diagnostics.js'

const identitySchema = z.object({
  id: z.string().uuid(),
  name: z.string().min(1).max(48),
  platform: z.enum(['darwin', 'win32']),
  fingerprint: z.string().regex(/^[a-f0-9]{64}$/),
  role: z.enum(['host', 'client']),
  localInput: z.number().int().min(0).max(65535),
  peerInput: z.number().int().min(0).max(65535),
  shortcut: z.string().regex(/^control\+alt\+(shift\+)?F(?:[1-9]|1[0-7])$/),
  clipboard: z.boolean(),
  clipboardMegabytes: z.number().int().min(1).max(32),
  deskflowPort: z.number().int().min(1024).max(65535),
  layoutVersion: z.literal(1).optional(),
  layoutSide: z.enum(['off', 'left', 'right', 'top', 'bottom']).default('off'),
  crossScreenVideo: z.enum(['follow', 'keep']).default('follow'),
  retainedDisplay: z.boolean().default(false),
})
type Identity = z.infer<typeof identitySchema>
export class Runtime extends EventEmitter {
  state!: Snapshot
  readonly display: DisplayController
  pointer?: {
    validate(): void
    hasRetainedDisplay(): boolean
    land(): Promise<void>
    crossToPeer?(position: number): Promise<void>
  }
  private engine: DeskflowEngine
  private server?: Server
  private link?: SecureChannel
  private flow?: SwitchFlow
  private retry?: NodeJS.Timeout
  private heartbeat?: NodeJS.Timeout
  private generation = 0
  private logSequence = 0
  private certificate = { fingerprint: '' }
  private engineTasks = Promise.resolve()
  private pendingEngine = false
  private commandInFlight = false
  private keyboardPending = false
  private cancelKeyboard?: () => void
  private publishTimer?: NodeJS.Timeout
  private accessibilityCheck?: Promise<boolean>
  private sessionShortcut = ''
  get switchShortcut() {
    return this.state.config.role === 'host' ? this.state.config.shortcut : this.sessionShortcut
  }
  get layout() {
    return this.state.config.role === 'host'
      ? { hostSide: this.state.config.layoutSide, video: this.state.config.crossScreenVideo }
      : this.state.sessionLayout
  }
  get pointerMode() {
    return !!this.layout && this.layout.hostSide !== 'off'
  }
  constructor(
    readonly store: Store,
    resources: string,
    private diagnostics?: DiagnosticLog,
  ) {
    super()
    this.display = new DisplayController(resources, (event, details) => this.trace(event, details))
    this.engine = new DeskflowEngine(store.dir)
  }
  async init() {
    await this.store.load()
    this.diagnostics?.protect(this.store.value.pairingCode)
    this.certificate = await this.engine.identity()
    const addresses = Object.values(networkInterfaces())
      .flatMap((list) => list ?? [])
      .filter((ip) => ip.family === 'IPv4' && !ip.internal)
      .map((ip) => ip.address)
    this.state = {
      cursorDisplays: [],
      sessionLayout: null,
      config: this.store.value.config,
      deviceId: this.store.value.id,
      platform: process.platform,
      addresses,
      monitors: [],
      running: false,
      listening: false,
      connected: false,
      engine: 'stopped',
      peer: null,
      activeDeviceId: null,
      displayDeviceId: null,
      switching: false,
      pairingCode: this.store.value.pairingCode,
      dependencies: { deskflow: false, displayControl: false, accessibility: false },
      logs: [],
      lastError: null,
      preview: false,
      diagnostics: this.diagnostics?.status ?? { directory: '', error: null },
    }
    this.engine.on('state', (engine: Snapshot['engine']) => {
      this.state.engine = engine
      this.trace('engine.state', { engine })
      this.publish()
    })
    this.engine.on('diagnostic', (message: string, error?: unknown) =>
      this.log('error', message, error),
    )
    this.engine.on('trace', (event: string, details: unknown) => this.trace(event, details))
    this.engine.on('switch', (change: Transition) => {
      void this.flow?.observe(change)
    })
    await this.refreshDependencies()
    this.log('info', '欢迎使用 Deskflow DDC。先配置显示器输入源，再配对另一台电脑。')
  }
  snapshot(): Snapshot {
    return structuredClone(this.state)
  }
  publish() {
    // Internal control listeners must react immediately; renderer snapshots are
    // batched so bursts of diagnostics do not clone/send the log array per line.
    this.emit('control-state')
    if (!this.publishTimer) {
      this.publishTimer = setTimeout(() => this.flushState(), 250)
      this.publishTimer.unref?.()
    }
  }
  private flushState() {
    clearTimeout(this.publishTimer)
    this.publishTimer = undefined
    if (this.diagnostics) this.state.diagnostics = this.diagnostics.status
    this.emit('state', this.snapshot())
  }
  private trace(event: string, details?: unknown) {
    this.diagnostics?.record('info', event, { session: this.generation, details })
  }
  log(level: 'info' | 'success' | 'error', message: string, error?: unknown) {
    message = this.diagnostics?.text(message) ?? message
    this.diagnostics?.record(level, 'activity', {
      session: this.generation,
      message,
      error,
      engine: this.state.engine,
      connected: this.state.connected,
    })
    if (level === 'error') this.state.lastError = message
    this.state.logs = [
      { id: ++this.logSequence, time: new Date().toISOString(), level, message },
      ...this.state.logs,
    ].slice(0, 160)
    this.publish()
  }
  refreshAccessibility(): Promise<boolean> {
    if (this.accessibilityCheck) return this.accessibilityCheck
    this.accessibilityCheck = this.display
      .checkAccessibility()
      .then((accessibility) => {
        if (this.state.dependencies.accessibility !== accessibility) {
          this.state.dependencies = { ...this.state.dependencies, accessibility }
          this.publish()
          this.trace('accessibility.checked', { accessibility })
        }
        return accessibility
      })
      .finally(() => {
        this.accessibilityCheck = undefined
      })
    return this.accessibilityCheck
  }
  async refreshDependencies() {
    const [deskflow, displayControl, accessibility] = await Promise.all([
      findDeskflow(this.state.config.deskflowPath),
      this.display.available(),
      this.refreshAccessibility(),
    ])
    this.state.dependencies = { deskflow: !!deskflow, displayControl, accessibility }
    this.publish()
    this.trace('dependencies.checked', this.state.dependencies)
  }
  async save(config: Config) {
    if (this.state.running) throw new Error('请先停止共享，再修改配置')
    const validated = configSchema.parse(config)
    // Save the role first so a former receiver can edit host-only settings.
    // Platform hotkey restrictions are checked when starting the engine.
    this.store.value.config = validated
    await this.store.save()
    this.state.config = validated
    await this.refreshDependencies()
    this.log('success', '配置已保存')
    this.trace('config.saved', diagnosticConfig(validated))
  }
  async scan() {
    if (this.state.switching) throw new Error('正在切换，请稍后扫描显示器')
    this.state.monitors = await this.display.list()
    this.publish()
    this.trace('display.scan', this.state.monitors)
    this.log(
      'info',
      this.state.monitors.length
        ? `发现 ${this.state.monitors.length} 台外接显示器`
        : '未发现可控制的外接显示器，请检查连接和显示器 DDC/CI 设置',
    )
  }
  async rotatePairing() {
    if (this.state.running) throw new Error('请先停止共享，再重新生成配对码')
    this.store.value.pairingCode = newPairingCode()
    this.store.value.trustedPeer = null
    await this.store.save()
    this.diagnostics?.protect(this.store.value.pairingCode)
    this.state.pairingCode = this.store.value.pairingCode
    this.state.peer = null
    this.publish()
    this.log('info', '已生成新的配对码，之前的设备需要重新配对')
  }
  private identity(): Identity {
    const c = this.state.config
    requireReady(c)
    return {
      id: this.state.deviceId,
      name: c.deviceName,
      platform: this.state.platform as 'darwin' | 'win32',
      fingerprint: this.certificate.fingerprint,
      role: c.role,
      localInput: c.localInput!,
      peerInput: c.peerInput!,
      shortcut: c.shortcut,
      clipboard: c.clipboard,
      clipboardMegabytes: c.clipboardMegabytes,
      deskflowPort: c.deskflowPort,
      layoutVersion: 1,
      layoutSide: c.layoutSide,
      crossScreenVideo: c.crossScreenVideo,
      retainedDisplay: this.pointer?.hasRetainedDisplay() ?? false,
    }
  }
  async start(pairingCode?: string) {
    if (!['darwin', 'win32'].includes(this.state.platform))
      throw new Error('当前支持 Apple Silicon macOS 和 Windows，暂不支持此操作系统')
    if (pairingCode) this.diagnostics?.protect(pairingCode.trim())
    if (this.state.running) throw new Error('共享已经启动')
    const c = this.state.config
    requireReady(c)
    requireSupportedShortcut(c, this.state.platform as NodeJS.Platform)
    await this.refreshDependencies()
    if (c.retainedDisplayId) {
      if (!this.pointer) throw new Error('屏幕定位服务不可用，请重新启动 Deskflow DDC')
      this.pointer.validate()
    }
    if (!this.state.dependencies.deskflow)
      throw new Error('请先安装 Deskflow 1.27.x 或选择 deskflow-core 可执行文件')
    if (!this.state.dependencies.displayControl)
      throw new Error('原生显示器控制程序不可用，请检查安装包或运行 pnpm build:native')
    await this.engine.assertAvailable(c)
    if (c.role === 'client') {
      if (!c.peerAddress) throw new Error('请填写键鼠提供端的局域网地址')
      const code = pairingCode?.trim() || this.store.value.pairingCode
      decodePairingCode(code)
      if (code !== this.store.value.pairingCode) {
        this.store.value.pairingCode = code
        this.store.value.trustedPeer = null
        await this.store.save()
      }
    }
    this.state.pairingCode = this.store.value.pairingCode
    this.state.running = true
    this.state.lastError = null
    const gen = ++this.generation
    this.trace('sharing.start', { config: diagnosticConfig(c), deviceId: this.state.deviceId })
    try {
      if (c.role === 'host') {
        const server = createServer((socket) => {
          if (gen !== this.generation || this.link) {
            socket.destroy()
            return
          }
          this.attach(
            new SecureChannel(socket, decodePairingCode(this.store.value.pairingCode), true),
            gen,
          )
        })
        this.server = server
        await new Promise<void>((resolve, reject) => {
          server.once('error', reject)
          server.listen(c.controlPort, '0.0.0.0', () => {
            server.removeListener('error', reject)
            resolve()
          })
        })
        server.on('error', (error) => this.log('error', `控制连接错误：${error.message}`, error))
        this.state.listening = true
        this.log('info', `等待另一台电脑连接，控制端口 ${c.controlPort}`)
      } else this.connectClient(gen)
      this.publish()
    } catch (e) {
      await this.stop()
      throw e
    }
  }
  private connectClient(gen: number) {
    if (gen !== this.generation || !this.state.running) return
    const c = this.state.config
    const socket = connect({ host: c.peerAddress, port: c.controlPort })
    this.attach(
      new SecureChannel(socket, decodePairingCode(this.store.value.pairingCode), false),
      gen,
    )
    this.log('info', `正在连接 ${c.peerAddress}:${c.controlPort}`)
  }
  private attach(link: SecureChannel, gen: number) {
    this.link = link
    let peerIdentity: Identity | undefined
    let identitySaved: Promise<void> | undefined
    const isCurrent = () => gen === this.generation && this.link === link && link.ready
    this.trace('control.attached', { address: link.remoteAddress })
    link.onRequest(async (method, payload) => {
      if (!isCurrent()) throw new Error('共享会话已结束')
      if (method === 'identify') {
        if (peerIdentity) throw new Error('已完成设备识别')
        const id = identitySchema.parse(payload)
        if (id.id === this.state.deviceId || id.role === this.state.config.role)
          throw new Error('两台电脑必须选择不同的键鼠角色')
        const pin = this.store.value.trustedPeer
        if (pin && (pin.id !== id.id || pin.fingerprint !== id.fingerprint))
          throw new Error('对端身份发生变化，请停止共享并重新生成配对码')
        if (
          id.localInput !== this.state.config.peerInput ||
          id.peerInput !== this.state.config.localInput
        )
          throw new Error('两端输入源映射不对应，请核对本机与对端的输入源数值')
        const hostLayout =
          this.state.config.role === 'host' ? this.state.config.layoutSide : id.layoutSide
        if (hostLayout !== 'off' && id.layoutVersion !== 1)
          throw new Error('鼠标跨屏需要两端都更新到支持通用布局的 Deskflow DDC 版本')
        peerIdentity = id
        this.store.value.trustedPeer = { id: id.id, fingerprint: id.fingerprint }
        identitySaved = this.store.save()
        await identitySaved
        if (!isCurrent()) throw new Error('共享会话已结束')
        this.state.peer = {
          id: id.id,
          name: id.name,
          platform: id.platform,
          fingerprint: id.fingerprint,
          inputConfigured: true,
          retainedDisplay: id.retainedDisplay,
        }
        this.state.sessionLayout =
          this.state.config.role === 'host'
            ? this.layout
            : { hostSide: id.layoutSide, video: id.crossScreenVideo }
        this.sessionShortcut = this.state.config.role === 'client' ? id.shortcut : ''
        this.state.connected = true
        this.publish()
        this.trace('identity.saved', { peer: this.state.peer })
        return this.identity()
      }
      if (!peerIdentity) throw new Error('尚未识别对端设备')
      if (method === 'ping') return 'pong'
      if (method === 'set-input') {
        const targetId = z.string().uuid().parse(payload)
        if (targetId !== this.state.deviceId && targetId !== peerIdentity.id)
          throw new Error('无效的切换目标')
        const c = this.state.config
        await this.display.setInput(
          c.displayId,
          targetId === this.state.deviceId ? c.localInput! : c.peerInput!,
        )
        this.log('info', '已发送对端请求的显示器切换指令')
        return true
      }
      if (method === 'land-pointer') {
        if (
          this.state.config.role !== 'client' ||
          !this.pointerMode ||
          this.layout?.video !== 'keep' ||
          !this.pointer?.hasRetainedDisplay()
        )
          throw new Error('当前设备没有配置可独立使用的屏幕')
        await this.pointer.land()
        return true
      }
      if (method === 'cross-screen') {
        if (this.state.config.role !== 'host' || !this.pointerMode)
          throw new Error('鼠标跨屏尚未启用')
        // The receiver may only request its own outbound crossing.
        if (this.commandInFlight || this.state.activeDeviceId !== peerIdentity.id)
          throw new Error('当前不能跨屏')
        await this.keyboardSwitch(this.state.deviceId)
        return true
      }
      if (method === 'switch') {
        if (this.state.config.role !== 'host') throw new Error('只有键鼠提供端可以协调切换')
        await this.commandSwitch(z.string().uuid().parse(payload))
        return true
      }
      throw new Error('不支持的控制请求')
    })
    link.on('ready', () => {
      void (async () => {
        const response = identitySchema.parse(await link.request('identify', this.identity()))
        if (
          !peerIdentity ||
          !identitySaved ||
          peerIdentity.id !== response.id ||
          peerIdentity.fingerprint !== response.fingerprint
        )
          throw new Error('双向设备识别未完成')
        // The remote reply can arrive while our inbound identify handler is saving its pin.
        // Start from this session's authenticated identity only after that save completes.
        const peer = peerIdentity
        await identitySaved
        if (!isCurrent()) return
        const enginePeer: Peer = {
          id: peer.id,
          name: peer.name,
          platform: peer.platform,
          fingerprint: peer.fingerprint,
          inputConfigured: true,
          retainedDisplay: peer.retainedDisplay,
        }
        this.log('success', `已与 ${peer.name} 建立加密控制连接`)
        const c = this.state.config
        const engineConfig =
          c.role === 'client'
            ? {
                ...c,
                shortcut: peer.shortcut,
                clipboard: peer.clipboard,
                clipboardMegabytes: peer.clipboardMegabytes,
                deskflowPort: peer.deskflowPort,
              }
            : c
        const hostId = c.role === 'host' ? this.state.deviceId : peer.id
        this.state.activeDeviceId = hostId
        if (c.role === 'host') this.setupFlow(peer.id)
        this.pendingEngine = true
        this.engineTasks = this.engineTasks
          .then(async () => {
            if (gen !== this.generation || this.link !== link || !link.ready) return
            await this.engine.start(
              engineConfig,
              this.state.deviceId,
              enginePeer,
              c.role === 'client' ? c.peerAddress : '',
            )
          })
          .catch((error) => {
            if (isCurrent()) {
              this.state.engine = 'error'
              this.log('error', error.message, error)
            }
          })
          .finally(() => {
            if (isCurrent()) this.pendingEngine = false
          })
        await this.engineTasks
        if (gen !== this.generation || this.link !== link) return
        this.heartbeat = setInterval(() => {
          void link.request('ping', null, 5000).catch(() => link.close())
        }, 4000)
      })().catch((error) => {
        if (isCurrent()) this.log('error', error.message, error)
        link.close()
      })
    })
    link.on('event', (name: string, payload: unknown) => {
      if (!peerIdentity || gen !== this.generation) return
      if (name === 'display-state' && this.state.config.role === 'client') {
        const state = z
          .object({
            active: z.string().uuid().nullable(),
            display: z.string().uuid().nullable().optional(),
            busy: z.boolean(),
            error: z.string().optional(),
            displayStatus: z.enum(['uninitialized', 'commanded', 'uncertain']).optional(),
            lastDisplay: z.string().uuid().nullable().optional(),
          })
          .safeParse(payload)
        if (state.success) {
          this.state.activeDeviceId = state.data.active
          this.state.displayDeviceId = state.data.display ?? null
          this.state.displayStatus =
            state.data.displayStatus ?? (state.data.display ? 'commanded' : 'uninitialized')
          this.state.lastDisplayDeviceId = state.data.lastDisplay ?? state.data.display ?? null
          this.state.switching = state.data.busy
          if (state.data.error) this.log('error', state.data.error)
          this.publish()
        }
      }
    })
    link.on('fault', (message: string) => {
      if (gen === this.generation) this.log('error', message)
    })
    link.on('closed', () => {
      if (this.link !== link || gen !== this.generation) return
      this.cancelKeyboard?.()
      this.link = undefined
      this.flow?.stop()
      this.flow = undefined
      clearInterval(this.heartbeat)
      this.state.connected = false
      this.state.sessionLayout = null
      this.sessionShortcut = ''
      this.state.peer = null
      this.state.activeDeviceId = null
      this.state.displayDeviceId = null
      this.state.switching = false
      this.pendingEngine = false
      this.state.displayStatus = 'uninitialized'
      this.state.lastDisplayDeviceId = null
      this.engineTasks = this.engineTasks
        .then(async () => {
          try {
            await this.engine.stop()
          } finally {
            await this.display.dispose?.()
          }
        })
        .catch((error) => this.log('error', error.message, error))
      this.log('error', '对端连接已断开，键鼠共享已停止。若画面未恢复，请用显示器按键选择输入源。')
      if (this.state.running && this.state.config.role === 'client')
        this.retry = setTimeout(() => this.connectClient(gen), 5000)
    })
  }
  private setupFlow(peerId: string) {
    const localId = this.state.deviceId
    this.flow = new SwitchFlow(new Set([localId, peerId]), {
      writeInput: async (from, to) => {
        if (this.pointerMode) {
          // Explicit video commands perform their write after the keyboard transition.
          if (this.commandInFlight) return
          const retained =
            to === localId ? this.pointer?.hasRetainedDisplay() : this.state.peer?.retainedDisplay
          const keepVideo =
            this.layout?.video === 'keep' &&
            retained &&
            this.state.displayStatus !== 'uncertain' &&
            this.state.displayDeviceId !== null
          if (!keepVideo && this.state.displayDeviceId !== to) {
            await this.writeDisplay(this.state.displayDeviceId ?? from, to)
          }
          if (this.state.displayDeviceId !== to && keepVideo) {
            this.trace('pointer.keyboardOnly', {
              from,
              to,
              displayDeviceId: this.state.displayDeviceId,
            })
            if (to === localId) await this.pointer!.land()
            else await this.link!.request('land-pointer', null, 10000)
          }
        } else {
          await this.writeDisplay(this.state.displayDeviceId ?? from, to)
        }
      },
      restoreKeyboard: (id) =>
        new Promise<void>((resolve, reject) => {
          const cleanup = () => {
            clearTimeout(timer)
            this.engine.removeListener('switch', observed)
          }
          const observed = (change: Transition) => {
            if (change.to === id) {
              cleanup()
              resolve()
            }
          }
          const timer = setTimeout(() => {
            cleanup()
            reject(new Error('未收到键鼠恢复确认'))
          }, 2500)
          this.engine.on('switch', observed)
          this.display.injectTarget(id === localId ? 'host' : 'peer').catch((error) => {
            cleanup()
            reject(error)
          })
        }),
      changed: (active, busy, error) => {
        this.state.activeDeviceId = active
        this.state.switching = busy
        if (error) this.log('error', error)
        this.publishControlState(error)
      },
      finished: (change, error) => {
        if (!error)
          this.log(
            'success',
            this.pointerMode
              ? `键鼠已切到${change.to === localId ? '本机' : '另一台电脑'}`
              : `已向显示器发送切到${change.to === localId ? '本机' : '另一台电脑'}的指令；请以实际画面为准`,
          )
        this.emit('transition', change, error)
      },
      failureLabel: this.pointerMode ? '键鼠跨屏失败' : undefined,
    })
  }
  private publishControlState(error?: string) {
    this.link?.notify('display-state', {
      active: this.state.activeDeviceId,
      display: this.state.displayDeviceId,
      displayStatus: this.state.displayStatus,
      lastDisplay: this.state.lastDisplayDeviceId,
      busy: this.state.switching || this.commandInFlight,
      error,
    })
    this.publish()
  }
  private async writeDisplay(from: string, to: string) {
    this.requireDisplaySource()
    const gen = this.generation,
      link = this.link,
      c = this.state.config
    try {
      if (from === this.state.deviceId)
        await this.display.setInput(
          c.displayId,
          to === this.state.deviceId ? c.localInput! : c.peerInput!,
        )
      else await link!.request('set-input', to)
    } catch (error) {
      if (gen === this.generation && this.link === link && this.state.connected) {
        this.state.lastDisplayDeviceId = this.state.displayDeviceId ?? from
        this.state.displayDeviceId = null
        this.state.displayStatus = 'uncertain'
      }
      throw error
    }
    if (gen !== this.generation || this.link !== link || !this.state.connected)
      throw new Error('共享会话已结束')
    this.state.displayDeviceId = to
    this.state.lastDisplayDeviceId = to
    this.state.displayStatus = 'commanded'
  }
  private requireDisplaySource() {
    if (this.state.displayStatus === 'uncertain') {
      throw new Error(
        '外接屏输入状态不确定，已暂停自动切屏；请停止共享，手动将画面切到键鼠提供端后重新连接',
      )
    }
  }
  private async keyboardSwitch(targetId: string, cross?: () => Promise<void>) {
    if (!this.state.connected || this.state.engine !== 'ready' || this.pendingEngine)
      throw new Error('键鼠共享尚未就绪')
    if (this.state.switching || this.keyboardPending) throw new Error('正在切换，请稍后')
    if (targetId !== this.state.deviceId && targetId !== this.state.peer?.id)
      throw new Error('无效的目标电脑')
    if (targetId === this.state.activeDeviceId) return
    this.trace('switch.requested', { targetId, activeDeviceId: this.state.activeDeviceId })
    this.keyboardPending = true
    try {
      await new Promise<void>((resolve, reject) => {
        let observed = false
        const onSwitch = (change: Transition) => {
          if (change.to === targetId) {
            observed = true
            this.trace('switch.keyboardObserved', change)
          }
        }
        this.engine.on('switch', onSwitch)
        const timer = setTimeout(() => {
          this.trace('switch.timeout', {
            targetId,
            keyboardObserved: observed,
            accessibility: this.state.dependencies.accessibility,
            engine: this.state.engine,
          })
          cleanup()
          reject(
            new Error(
              observed
                ? this.pointerMode
                  ? '已收到 Deskflow 键鼠切换，但鼠标落点或显示器尚未完成，请查看 pointer 日志'
                  : '已收到 Deskflow 键鼠切换，但显示器处理尚未完成，请查看 DDC 和对端日志'
                : '未收到 Deskflow 键鼠切换记录，请查看快捷键发送与引擎日志；此提示不代表辅助功能权限被拒绝',
            ),
          )
        }, 12000)
        const listener = (change: Transition, error?: string) => {
          if (change.to === targetId) {
            cleanup()
            error ? reject(new Error(error)) : resolve()
          }
        }
        const cleanup = () => {
          clearTimeout(timer)
          this.removeListener('transition', listener)
          this.engine.removeListener('switch', onSwitch)
          this.cancelKeyboard = undefined
        }
        this.cancelKeyboard = () => {
          cleanup()
          reject(new Error('共享会话已结束'))
        }
        this.on('transition', listener)
        const send =
          cross ??
          (() => this.display.injectTarget(targetId === this.state.deviceId ? 'host' : 'peer'))
        send().catch((error) => {
          cleanup()
          reject(error)
        })
      })
    } finally {
      this.keyboardPending = false
    }
  }
  private async commandSwitch(targetId: string) {
    if (this.commandInFlight) throw new Error('正在切换，请稍后')
    this.requireDisplaySource()
    if (!this.pointerMode) {
      await this.keyboardSwitch(targetId)
      return
    }
    const gen = this.generation,
      from = this.state.activeDeviceId
    const source = this.state.displayDeviceId ?? from
    let writeStarted = false
    this.commandInFlight = true
    try {
      await this.keyboardSwitch(targetId)
      if (gen !== this.generation || !this.state.connected) throw new Error('共享会话已结束')
      this.state.switching = true
      this.publishControlState()
      // Keyboard ownership may differ from the shared display input.
      // Send DDC through the last commanded video source, not through keyboard ownership.
      writeStarted = true
      await this.writeDisplay(source!, targetId)
      this.log('success', '已发送外接屏输入切换指令；请以实际画面为准')
    } catch (error) {
      if (gen === this.generation && this.state.connected) {
        if (writeStarted) {
          this.state.switching = false
          if (from && from !== this.state.activeDeviceId) {
            try {
              await this.keyboardSwitch(from)
            } catch (restoreError) {
              this.trace('pointer.restoreFailed', { error: restoreError })
            }
          }
        }
        this.trace('display.explicitSwitchFailed', { targetId, error })
      }
      throw error
    } finally {
      this.commandInFlight = false
      if (gen === this.generation) {
        this.state.switching = false
        this.publishControlState()
      }
    }
  }
  async switchKeyboardToPeer(position?: number) {
    if (
      !this.pointerMode ||
      this.commandInFlight ||
      this.state.activeDeviceId !== this.state.deviceId ||
      !this.state.peer
    )
      return
    if (this.state.config.role === 'host') {
      const pointer = this.pointer
      if (position !== undefined && pointer?.crossToPeer) {
        if (!Number.isFinite(position) || position < 0 || position > 1)
          throw new Error('跨屏位置无效')
        this.trace('pointer.edgeCrossing', { position, side: this.layout?.hostSide })
        await this.keyboardSwitch(this.state.peer.id, () => pointer.crossToPeer!(position))
      } else await this.keyboardSwitch(this.state.peer.id)
    } else await this.link!.request('cross-screen', null, 15000)
  }
  async toggleDisplay(source: 'button' | 'shortcut' = 'button') {
    this.trace('display.toggleRequested', {
      source,
      activeDeviceId: this.state.activeDeviceId,
      displayDeviceId: this.state.displayDeviceId,
      switching: this.state.switching,
      commandInFlight: this.commandInFlight,
      keyboardPending: this.keyboardPending,
    })
    const local = this.state.displayDeviceId === this.state.deviceId
    await this.switchTo(
      local ||
        (this.state.displayDeviceId === null && this.state.activeDeviceId === this.state.deviceId)
        ? 'peer'
        : 'local',
    )
  }
  async switchTo(target: 'local' | 'peer') {
    const id = target === 'local' ? this.state.deviceId : this.state.peer?.id
    if (!id) throw new Error('另一台电脑尚未连接')
    if (this.state.config.role === 'host') await this.commandSwitch(id)
    else await this.link!.request('switch', id, 15000)
  }
  async testInput(value: number) {
    if (this.state.running) throw new Error('请先停止共享，再单独测试显示器')
    const c = this.state.config
    requireReady(c)
    if (value !== c.localInput && value !== c.peerInput) throw new Error('只能测试已保存的输入源')
    await this.display.setInput(c.displayId, value)
    this.log('info', `DDC 输入源 ${value} 的写入指令已发送，请确认物理画面`)
  }
  async stop() {
    this.trace('sharing.stop')
    this.cancelKeyboard?.()
    ++this.generation
    clearTimeout(this.retry)
    clearInterval(this.heartbeat)
    this.flow?.stop()
    this.state.running = false
    this.state.listening = false
    this.state.connected = false
    this.state.sessionLayout = null
    this.sessionShortcut = ''
    this.state.peer = null
    this.link?.close()
    this.link = undefined
    if (this.server) {
      this.server.close()
      this.server = undefined
    }
    try {
      await this.engineTasks
      await this.engine.stop()
    } finally {
      await this.display.dispose?.()
    }
    this.flow = undefined
    this.state.engine = 'stopped'
    this.state.switching = false
    this.state.activeDeviceId = null
    this.state.displayDeviceId = null
    this.pendingEngine = false
    this.state.displayStatus = 'uninitialized'
    this.state.lastDisplayDeviceId = null
    this.flushState()
  }
}
