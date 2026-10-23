export function closeMainWindow(
  state: { quitting: boolean; running: boolean; hasTray: boolean },
  event: { preventDefault(): void },
  hide: () => void,
  quit: () => void,
) {
  if (state.quitting) return
  if (state.running && state.hasTray) {
    event.preventDefault()
    hide()
    return
  }
  // Keep the window alive until before-quit has awaited Runtime.stop().
  event.preventDefault()
  quit()
}

export function assertRendererSender(
  event: { sender: unknown; senderFrame: unknown },
  window:
    | {
        isDestroyed(): boolean
        webContents: { mainFrame: unknown }
      }
    | undefined,
) {
  if (
    !window ||
    window.isDestroyed() ||
    event.sender !== window.webContents ||
    event.senderFrame !== window.webContents.mainFrame
  ) {
    throw new Error('无效的应用请求')
  }
}
