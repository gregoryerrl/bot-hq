import { globalShortcut, BrowserWindow } from 'electron'

export function registerHotkeys(window: BrowserWindow) {
  // Push-to-talk: Cmd+Shift+Space
  globalShortcut.register('CommandOrControl+Shift+Space', () => {
    window.webContents.send('hotkey:push-to-talk')
    window.show()
  })
}

export function unregisterHotkeys() {
  globalShortcut.unregisterAll()
}
