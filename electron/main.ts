import { app, BrowserWindow, ipcMain } from 'electron'
import { join } from 'path'
import { registerHotkeys, unregisterHotkeys } from './hotkey'
import { createTray } from './tray'
import { setupAudioIPC } from './audio'

let mainWindow: BrowserWindow | null = null

function createWindow() {
  mainWindow = new BrowserWindow({
    width: 380,
    height: 520,
    alwaysOnTop: true,
    frame: false,
    transparent: true,
    resizable: true,
    skipTaskbar: false,
    webPreferences: {
      preload: join(__dirname, '../preload/index.js'),
      contextIsolation: true,
      nodeIntegration: false
    }
  })

  if (process.env.ELECTRON_RENDERER_URL) {
    mainWindow.loadURL(process.env.ELECTRON_RENDERER_URL)
  } else {
    mainWindow.loadFile(join(__dirname, '../renderer/index.html'))
  }
}

app.whenReady().then(() => {
  createWindow()

  if (mainWindow) {
    registerHotkeys(mainWindow)
    createTray(mainWindow)
    setupAudioIPC(mainWindow)
  }

  // Placeholder for push-to-talk release from renderer
  ipcMain.on('hotkey:push-to-talk-release', () => {
    // Will be implemented when voice pipeline is ready
  })
})

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') app.quit()
})

app.on('will-quit', () => {
  unregisterHotkeys()
})
