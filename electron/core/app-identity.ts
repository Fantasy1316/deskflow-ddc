import { existsSync } from 'node:fs'
import { join } from 'node:path'

const productName = 'Deskflow DDC'
interface ApplicationPaths {
  getPath(name: 'appData'): string
  setPath(name: 'userData', path: string): void
  setName(name: string): void
}

export function configureApplicationIdentity(app: ApplicationPaths, platform = process.platform) {
  const appData = app.getPath('appData')
  const existingName = [productName, 'ScreenBridge', 'screenbridge'].find((name) =>
    existsSync(join(appData, name, 'screenbridge-v1', 'settings.json')),
  )
  // Electron captures the macOS keychain service/account before app.ready.
  // Reusing the directory alone cannot decrypt a former product's safeStorage data.
  const legacyKeychain = platform === 'darwin' && existingName && existingName !== productName
  app.setName(legacyKeychain ? 'ScreenBridge' : productName)
  app.setPath('userData', join(appData, existingName ?? productName))
  // Restoring the visible name after ready does not change the captured keychain namespace.
  return () => app.setName(productName)
}
