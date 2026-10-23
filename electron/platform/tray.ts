import { execFile } from 'node:child_process'
import { join } from 'node:path'
import { nativeImage, nativeTheme } from 'electron'

/** Windows taskbar theme can differ from the theme used by applications. */
async function darkTaskbar() {
  try {
    const output = await new Promise<string>((resolve, reject) => {
      execFile(
        'reg.exe',
        [
          'query',
          'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize',
          '/v',
          'SystemUsesLightTheme',
        ],
        { windowsHide: true, timeout: 2000, maxBuffer: 8192 },
        (error, stdout) => (error ? reject(error) : resolve(stdout)),
      )
    })
    const match = output.match(/SystemUsesLightTheme\s+REG_DWORD\s+0x([0-9a-f]+)/i)
    if (match) return parseInt(match[1]!, 16) === 0
  } catch {
    /* Older Windows versions may not expose this value. */
  }
  return nativeTheme.shouldUseDarkColors
}
export async function trayImage(resources: string) {
  const file =
    process.platform === 'darwin'
      ? 'trayTemplate.png'
      : (await darkTaskbar())
        ? 'tray-white.ico'
        : 'tray-black.ico'
  const image = nativeImage.createFromPath(join(resources, 'icons', file))
  if (process.platform === 'darwin') image.setTemplateImage(true)
  return image
}
