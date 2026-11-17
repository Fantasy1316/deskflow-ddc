import type { Config, DdcProfile, InputProtocol, Monitor, Snapshot } from './types'

export const protocolName = (protocol: InputProtocol) =>
  protocol === 'input-alt' ? '兼容输入控制（部分 LG）' : '标准输入控制'
export const effectiveProtocol = (config: Config): InputProtocol =>
  config.ddcProtocol === 'auto' ? config.ddcAutoProtocol : config.ddcProtocol
export const mappingSignature = (c: Config) =>
  JSON.stringify([c.displayMethod, c.displayId, effectiveProtocol(c), c.localInput, c.peerInput])
export function savedDdcProfile(state: Snapshot, monitor?: Monitor): DdcProfile | undefined {
  if (!monitor?.identity) return undefined
  return state.ddcProfiles?.find(
    (p) =>
      p.identity === monitor.identity &&
      p.platform === state.platform &&
      (p.protocol === 'standard' || state.platform === 'darwin'),
  )
}
export function inputPresets(protocol: InputProtocol) {
  return protocol === 'input-alt'
    ? [
        { value: 144, port: 'hdmi-1', name: 'HDMI 1' },
        { value: 145, port: 'hdmi-2', name: 'HDMI 2' },
        { value: 208, port: 'dp-1', name: 'DisplayPort 1' },
        { value: 209, port: 'dp-2', name: 'DisplayPort 2' },
        { value: 210, port: 'usb-c', name: 'USB-C / DisplayPort 3' },
      ]
    : [
        { value: 17, port: 'hdmi-1', name: 'HDMI 1' },
        { value: 18, port: 'hdmi-2', name: 'HDMI 2' },
        { value: 15, port: 'dp-1', name: 'DisplayPort 1' },
        { value: 16, port: 'dp-2', name: 'DisplayPort 2' },
        { value: 27, port: 'usb-c', name: 'USB-C' },
        { value: 3, port: 'dvi-1', name: 'DVI 1' },
        { value: 4, port: 'dvi-2', name: 'DVI 2' },
        { value: 1, port: 'vga', name: 'VGA' },
      ]
}
