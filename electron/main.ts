import { app, BrowserWindow, ipcMain } from 'electron'
import { join } from 'path'
import { readFileSync } from 'fs'
import { registerHotkeys, unregisterHotkeys } from './hotkey'
import { createTray, updateTrayStatus } from './tray'
import { setupAudioIPC } from './audio'
import { GeminiSession } from './gemini/session'
import { toolToFunctionDeclaration } from './gemini/types'
import { registerAllTools, toolRegistry } from './tools/index'
import { ToolDispatcher } from './tools/dispatcher'
import { buildSystemInstruction } from './memory/context'
import { getConfig, updateConfig } from './config'
import type { AgentState } from './gemini/types'

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
let dispatcher: ToolDispatcher | null = null

function createWindow() {
  const cfg = getConfig()
  mainWindow = new BrowserWindow({
    width: 380,
    height: 520,
    alwaysOnTop: cfg.alwaysOnTop,
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

async function initGemini(window: BrowserWindow) {
  const env = loadEnv()
  if (!env.GEMINI_API_KEY) {
    console.warn('GEMINI_API_KEY not found in .env — Gemini session will not start')
    return
  }

  // 1. Register all tools
  registerAllTools()
  const allTools = toolRegistry.getAll()
  console.log(`Registered ${allTools.length} tools`)

  // 2. Build system instruction from memory + project context
  let systemInstruction: string
  try {
    systemInstruction = await buildSystemInstruction()
  } catch (err) {
    console.warn('Failed to build system instruction from DB, using fallback:', err)
    systemInstruction =
      'You are Bot-HQ, a voice-controlled computer agent. Be concise in voice responses.'
  }

  // 3. Convert tool definitions to Gemini FunctionDeclarations
  const functionDeclarations = allTools.map(toolToFunctionDeclaration)

  // 4. Create the Gemini session
  geminiSession = new GeminiSession(
    {
      apiKey: env.GEMINI_API_KEY,
      systemInstruction,
      tools: functionDeclarations
    },
    window
  )

  // 5. Create the tool dispatcher
  dispatcher = new ToolDispatcher(window)

  // 6. Set the tool-call handler: Gemini -> Dispatcher -> Gemini
  geminiSession.setToolCallHandler(async (calls) => {
    if (!dispatcher) {
      return calls.map((c) => ({
        id: c.id,
        name: c.name,
        response: { error: 'Dispatcher not initialized' }
      }))
    }
    return dispatcher.dispatch(calls)
  })

  // 7. Update tray status on agent state changes
  geminiSession.setStateChangeHandler((state: AgentState) => {
    updateTrayStatus(state)
  })

  // 8. Connect to Gemini Live API
  try {
    await geminiSession.connect()
    console.log('Gemini Live session connected')
  } catch (err) {
    console.error('Gemini connection failed:', err)
  }
}

app.whenReady().then(() => {
  createWindow()

  if (mainWindow) {
    registerHotkeys(mainWindow)
    createTray(mainWindow)
    setupAudioIPC(mainWindow)

    // Initialize Gemini with tools, dispatcher, and context
    initGemini(mainWindow).catch((err) =>
      console.error('Gemini init failed:', err)
    )
  }

  // Forward audio chunks from renderer to Gemini
  ipcMain.on('audio:chunk', (_event, base64PCM: string) => {
    geminiSession?.sendAudio(base64PCM)
  })

  // Placeholder for push-to-talk release from renderer
  ipcMain.on('hotkey:push-to-talk-release', () => {
    // Will be implemented when voice pipeline is ready
  })

  // Config IPC handlers
  ipcMain.handle('config:get', () => {
    return getConfig()
  })

  ipcMain.handle('config:update', (_event, updates: Record<string, unknown>) => {
    const updated = updateConfig(updates)
    // Apply alwaysOnTop immediately if it changed
    if ('alwaysOnTop' in updates && mainWindow) {
      mainWindow.setAlwaysOnTop(updated.alwaysOnTop)
    }
    return updated
  })
})

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') app.quit()
})

app.on('will-quit', () => {
  unregisterHotkeys()
  geminiSession?.disconnect().catch(() => {})
})
