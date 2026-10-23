import {
  app,
  BrowserWindow,
  ipcMain,
  dialog,
  shell,
  Tray,
  Menu,
  clipboard,
  nativeTheme,
} from 'electron'
import { join, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'
import { existsSync } from 'node:fs'
import { release } from 'node:os'
import { z } from 'zod'
import { Runtime } from './core/runtime.js'
import { Store } from './core/store.js'
import { PointerBridge } from './platform/pointer.js'
import { trayImage } from './platform/tray.js'
import { actionSchema, unreachableAction } from '../src/shared/actions.js'
import { closeMainWindow, assertRendererSender } from './core/window-lifecycle.js'
import { DiagnosticLog, diagnosticSnapshot } from './core/diagnostics.js'
import { configureApplicationIdentity } from './core/app-identity.js'

const restoreProductName = configureApplicationIdentity(app)

const here = dirname(fileURLToPath(import.meta.url))
let window: BrowserWindow | undefined, tray: Tray | undefined, runtime: Runtime | undefined
let quitting = false
let diagnostics: DiagnosticLog | undefined,
  exporting = false
const environment = () => ({
  appVersion: app.getVersion(),
  platform: process.platform,
  arch: process.arch,
  osRelease: release(),
  electron: process.versions.electron,
  node: process.versions.node,
  timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
})
async function exportDiagnostics() {
  if (!diagnostics || exporting) return
  exporting = true
  try {
    const options: Electron.SaveDialogOptions = {
      title: '导出诊断日志',
      defaultPath: join(
        app.getPath('downloads'),
        `deskflow-ddc-${process.platform}-${new Date().toISOString().replace(/[:.]/g, '-')}.json`,
      ),
      filters: [{ name: '诊断日志', extensions: ['json'] }],
    }
    const result =
      window && !window.isDestroyed()
        ? await dialog.showSaveDialog(window, options)
        : await dialog.showSaveDialog(options)
    if (result.canceled || !result.filePath) return
    diagnostics.record('info', 'diagnostics.export')
    diagnostics.exportTo(result.filePath, {
      environment: environment(),
      snapshot: runtime?.state ? diagnosticSnapshot(runtime.snapshot()) : null,
    })
    if (runtime?.state) runtime.log('success', '诊断日志已导出，可以将该 JSON 文件发给排查人员。')
    shell.showItemInFolder(result.filePath)
  } finally {
    exporting = false
  }
}
// Monitor keeps Node's normal fatal-exception behavior; the last error is already on disk.
process.on('uncaughtExceptionMonitor', (error) =>
  diagnostics?.record('error', 'main.uncaughtException', error),
)
process.on('unhandledRejection', (error) => {
  diagnostics?.record('error', 'main.unhandledRejection', error)
  if (runtime?.state) {
    runtime.log('error', '发生未处理异常，已记录诊断日志并停止共享。', error)
    void runtime
      .stop()
      .catch((stopError) => diagnostics?.record('error', 'main.stopFailed', stopError))
  }
})
if (!app.requestSingleInstanceLock()) app.quit()
else {
  app.on('second-instance', () => {
    window?.show()
    window?.focus()
  })
  app
    .whenReady()
    .then(async () => {
      restoreProductName()
      if (process.platform === 'darwin') {
        // Electron's default menu may have captured the legacy keychain name
        // before ready. Rebuild it after restoring the visible product name.
        app.setAboutPanelOptions({ applicationName: 'Deskflow DDC' })
        Menu.setApplicationMenu(
          Menu.buildFromTemplate([
            {
              label: 'Deskflow DDC',
              submenu: [
                { role: 'about', label: 'About Deskflow DDC' },
                { type: 'separator' },
                { role: 'services' },
                { type: 'separator' },
                { role: 'hide', label: 'Hide Deskflow DDC' },
                { role: 'hideOthers' },
                { role: 'unhide' },
                { type: 'separator' },
                { role: 'quit', label: 'Quit Deskflow DDC' },
              ],
            },
            { role: 'fileMenu' },
            { role: 'editMenu' },
            { role: 'viewMenu' },
            { role: 'windowMenu' },
            { role: 'help', submenu: [] },
          ]),
        )
      }
      const dataDir = join(app.getPath('userData'), 'screenbridge-v1')
      diagnostics = new DiagnosticLog(join(dataDir, 'logs'))
      diagnostics.record('info', 'app.start', environment())
      const packagedResources = join(process.resourcesPath, 'native')
      const resources = existsSync(packagedResources)
        ? packagedResources
        : join(here, '../../resources')
      runtime = new Runtime(new Store(dataDir), resources, diagnostics)
      await runtime.init()
      runtime.pointer = new PointerBridge(runtime)
      window = new BrowserWindow({
        width: 1180,
        height: 850,
        minWidth: 960,
        minHeight: 720,
        title: 'Deskflow DDC',
        icon: join(resources, 'icons/app.png'),
        backgroundColor: '#f6f7f8',
        titleBarStyle: process.platform === 'darwin' ? 'hiddenInset' : 'default',
        webPreferences: {
          preload: join(here, 'preload.cjs'),
          contextIsolation: true,
          nodeIntegration: false,
          sandbox: true,
        },
      })
      if (process.platform === 'win32') window.removeMenu()
      if (process.platform === 'darwin') {
        window.on('focus', () => {
          void runtime!
            .refreshAccessibility()
            .catch((error) =>
              runtime!.log('error', '无法刷新辅助功能权限，请稍后重新检测。', error),
            )
        })
      }
      window.webContents.setWindowOpenHandler(() => ({ action: 'deny' }))
      window.webContents.on('will-navigate', (event) => event.preventDefault())
      const validSender = (event: Electron.IpcMainInvokeEvent) => {
        assertRendererSender(event, window)
      }
      ipcMain.handle('screenbridge:snapshot', (event) => {
        validSender(event)
        return runtime!.snapshot()
      })
      let errorWindow = Date.now(),
        rendererErrors = 0
      ipcMain.handle('screenbridge:renderer-error', (event, raw: unknown) => {
        validSender(event)
        const error = z
          .object({
            source: z.enum(['window.error', 'window.unhandledrejection', 'vue']),
            message: z.string().max(8192),
            stack: z.string().max(16384).optional(),
          })
          .strict()
          .parse(raw)
        if (Date.now() - errorWindow > 60_000) {
          errorWindow = Date.now()
          rendererErrors = 0
        }
        if (++rendererErrors > 10) {
          if (rendererErrors === 11)
            diagnostics!.record('error', 'renderer.errorsThrottled', {
              message: '一分钟内界面异常超过 10 条，暂缓重复记录',
            })
          return
        }
        diagnostics!.record('error', 'renderer.error', error)
        runtime!.log('error', '界面发生异常，详细信息已写入诊断日志。')
      })
      ipcMain.handle('screenbridge:action', async (event, raw: unknown) => {
        validSender(event)
        const action = actionSchema.parse(raw)
        diagnostics!.record('info', 'ui.action', { type: action.type })
        try {
          switch (action.type) {
            case 'save':
              await runtime!.save(action.config)
              break
            case 'scan':
              await runtime!.scan()
              break
            case 'start':
              await runtime!.start(action.pairingCode)
              break
            case 'stop':
              await runtime!.stop()
              runtime!.log('info', '共享已停止')
              break
            case 'switch':
              await runtime!.switchTo(action.target)
              break
            case 'testInput':
              await runtime!.testInput(action.value)
              break
            case 'rotatePairing':
              await runtime!.rotatePairing()
              break
            case 'copyPairing':
              clipboard.writeText(runtime!.snapshot().pairingCode)
              break
            case 'openAccessibility':
              await shell.openExternal(
                'x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility',
              )
              break
            case 'checkAccessibility': {
              const granted = await runtime!.refreshAccessibility()
              runtime!.log(
                granted ? 'success' : 'info',
                granted
                  ? '已检测到本应用的辅助功能授权。Deskflow 的共享权限需单独开启。'
                  : '未检测到本应用的辅助功能授权，请在系统设置中开启后重新检测。',
              )
              break
            }
            case 'openDataFolder':
              await shell.openPath(runtime!.store.dir)
              break
            case 'openLogFolder': {
              const error = await shell.openPath(diagnostics!.directory)
              if (error) throw new Error(error)
              break
            }
            case 'exportDiagnostics':
              await exportDiagnostics()
              break
            case 'chooseDeskflow': {
              if (runtime!.state.running) throw new Error('请先停止共享，再选择键鼠引擎')
              const result = await dialog.showOpenDialog(window!, {
                title: '选择 Deskflow 1.27.x 的 deskflow-core',
                properties: [
                  'openFile',
                  ...(process.platform === 'darwin' ? ['treatPackageAsDirectory' as const] : []),
                ],
              })
              if (result.filePaths[0])
                await runtime!.save({ ...runtime!.state.config, deskflowPath: result.filePaths[0] })
              break
            }
            default:
              unreachableAction(action)
          }
        } catch (error) {
          const message = error instanceof Error ? error.message : String(error)
          runtime!.log('error', message, error)
          throw new Error(diagnostics!.text(message))
        }
      })
      runtime.on('state', (state) => {
        if (!window?.isDestroyed()) window?.webContents.send('screenbridge:state', state)
      })
      window.webContents.on('render-process-gone', (_event, details) => {
        diagnostics!.record('error', 'renderer.processGone', details)
        void runtime!
          .stop()
          .catch((error) => diagnostics!.record('error', 'renderer.stopFailed', error))
      })
      window.webContents.on('preload-error', (_event, path, error) =>
        diagnostics!.record('error', 'renderer.preloadFailed', { path, error }),
      )
      app.on('child-process-gone', (_event, details) =>
        diagnostics!.record('error', 'app.childProcessGone', details),
      )
      if (process.env.DESKFLOW_DDC_DEV_URL && !app.isPackaged)
        await window.loadURL(process.env.DESKFLOW_DDC_DEV_URL)
      else await window.loadFile(join(here, '../../dist/index.html'))
      const icon = await trayImage(resources)
      if (!icon.isEmpty()) {
        tray = new Tray(icon)
        let themeRevision = 0
        nativeTheme.on('updated', () => {
          const revision = ++themeRevision
          void trayImage(resources)
            .then((image) => {
              if (revision === themeRevision && tray && !tray.isDestroyed() && !image.isEmpty())
                tray.setImage(image)
            })
            .catch((error) => diagnostics?.record('error', 'tray.themeUpdateFailed', error))
        })
        tray.setToolTip('Deskflow DDC')
        tray.setContextMenu(
          Menu.buildFromTemplate([
            { label: '打开 Deskflow DDC', click: () => window?.show() },
            {
              label: '切到本机',
              click: () =>
                runtime!.switchTo('local').catch((e) => runtime!.log('error', e.message, e)),
            },
            {
              label: '切到另一台电脑',
              click: () =>
                runtime!.switchTo('peer').catch((e) => runtime!.log('error', e.message, e)),
            },
            { type: 'separator' },
            {
              label: '停止共享',
              click: () => {
                void runtime!.stop().catch((e) => runtime!.log('error', e.message, e))
              },
            },
            {
              label: '导出诊断日志…',
              click: () => {
                void exportDiagnostics().catch((e) => runtime!.log('error', e.message, e))
              },
            },
            {
              label: '退出',
              click: () => {
                app.quit()
              },
            },
          ]),
        )
        tray.on('click', () => window?.show())
      }
      window.on('close', (event) =>
        closeMainWindow(
          { quitting, running: !!runtime?.state.running, hasTray: !!tray && !tray.isDestroyed() },
          event,
          () => window?.hide(),
          () => app.quit(),
        ),
      )
    })
    .catch(async (error) => {
      diagnostics?.record('error', 'app.startFailed', error)
      const result = await dialog.showMessageBox({
        type: 'error',
        title: 'Deskflow DDC 启动失败',
        message: diagnostics?.text(error.message) ?? error.message,
        buttons: diagnostics ? ['导出诊断日志', '退出'] : ['退出'],
        defaultId: 0,
        cancelId: diagnostics ? 1 : 0,
      })
      if (diagnostics && result.response === 0) {
        try {
          await exportDiagnostics()
        } catch (exportError) {
          diagnostics.record('error', 'diagnostics.exportFailed', exportError)
        }
      }
      if (runtime?.state) {
        try {
          await runtime.stop()
        } catch (stopError) {
          diagnostics?.record('error', 'app.startFailureCleanupFailed', stopError)
        }
      }
      quitting = true
      app.quit()
    })
  app.on('before-quit', (event) => {
    if (!quitting && runtime) {
      event.preventDefault()
      quitting = true
      void runtime
        .stop()
        .catch((error) => diagnostics?.record('error', 'app.stopFailed', error))
        .finally(() => {
          diagnostics?.record('info', 'app.quit')
          app.quit()
        })
    }
  })
  app.on('window-all-closed', () => {
    if (!runtime?.state.running) app.quit()
  })
  app.on('activate', () => window?.show())
}
