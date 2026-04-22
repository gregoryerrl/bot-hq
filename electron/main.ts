import { app, BrowserWindow, ipcMain } from 'electron'
import { join } from 'path'
import { readFileSync } from 'fs'
import { registerHotkeys, unregisterHotkeys } from './hotkey'
import { createTray } from './tray'
import { setupAudioIPC } from './audio'
import { GeminiSession } from './gemini/session'

function loadEnv(): Record<string, string> {
  try {
    const envPath = join(__dirname, '../../.env')
    const content = readFileSync(envPath, 'utf-8')
    const env: Record<string, string> = {}
    for (const line of content.split('\n')) {
      const trimmed = line.trim()
      if (!trimmed || trimmed.startsWith('#')) continue
      const eqIndex = trimmed.indexOf('=')
      if (eqIndex === -1) continue
      const key = trimmed.slice(0, eqIndex).trim()
      const value = trimmed.slice(eqIndex + 1).trim()
      if (key) env[key] = value
    }
    return env
  } catch {
    return {}
  }
}

let mainWindow: BrowserWindow | null = null
let geminiSession: GeminiSession | null = null

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

    // Connect to Gemini Live API
    const env = loadEnv()
    if (env.GEMINI_API_KEY) {
      geminiSession = new GeminiSession(
        {
          apiKey: env.GEMINI_API_KEY,
          systemInstruction:
            'You are Bot-HQ, a voice-controlled computer agent. Be concise in voice responses.',
          tools: [] // Tools will be added in Task 18
        },
        mainWindow
      )
      geminiSession
        .connect()
        .catch((err) =>
          console.error('Gemini connection failed:', err)
        )
    }
  }

  // Forward audio chunks from renderer to Gemini
  ipcMain.on('audio:chunk', (_event, base64PCM: string) => {
    geminiSession?.sendAudio(base64PCM)
  })

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
  geminiSession?.disconnect().catch(() => {})
})
