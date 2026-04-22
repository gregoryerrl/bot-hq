# Bot-HQ v3: Voice Agent Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build an Electron desktop app that lets you press a hotkey, speak to Gemini Live, and have it execute actions on your computer — file ops, shell commands, git, screenshots, memory, project focus, and Claude Code session management via tmux.

**Architecture:** Electron main process handles audio capture, Gemini Live WebSocket (via `@google/genai` SDK), tool execution, SQLite memory, and tmux session management. React renderer in a floating always-on-top window shows state, action log, and confirmation dialogs. System tray for status.

**Tech Stack:** Electron 35+, electron-vite, React 19, Tailwind CSS 4, @google/genai SDK, Gemini 3.1 Flash Live Preview, SQLite via better-sqlite3, Drizzle ORM, simple-git, Lucide React

---

## Phase 1: Foundation — Electron + Gemini Voice Loop

### Task 1: Scaffold Electron App

**Files:**
- Create: `package.json` (replace existing)
- Create: `electron/main.ts`
- Create: `electron/preload.ts`
- Create: `src/App.tsx`
- Create: `src/main.tsx`
- Create: `src/styles/globals.css`
- Create: `electron.vite.config.ts`
- Create: `tsconfig.json`
- Create: `tsconfig.node.json`
- Create: `tsconfig.web.json`

**Step 1: Clean out old Next.js project and initialize Electron + Vite**

Remove old src/, then scaffold. We use `electron-vite` for bundling both main and renderer.

```bash
# Remove old source files (keep docs/, data/, .env, .gitignore)
rm -rf src/ scripts/ drizzle/ public/
rm -f next.config.ts next-env.d.ts postcss.config.mjs tailwind.config.ts drizzle.config.ts components.json middleware.ts instrumentation.ts
```

**Step 2: Create package.json**

```json
{
  "name": "bot-hq",
  "version": "3.0.0",
  "private": true,
  "main": "out/main/index.js",
  "scripts": {
    "dev": "electron-vite dev",
    "build": "electron-vite build",
    "start": "electron-vite preview",
    "test": "vitest run",
    "test:watch": "vitest"
  },
  "dependencies": {
    "@google/genai": "^1.50.1",
    "better-sqlite3": "^12.6.2",
    "drizzle-orm": "^0.45.1",
    "lucide-react": "^0.562.0",
    "react": "^19.2.3",
    "react-dom": "^19.2.3",
    "simple-git": "^3.27.0",
    "uuid": "^11.1.0"
  },
  "devDependencies": {
    "@tailwindcss/vite": "^4.0.0",
    "@types/better-sqlite3": "^7.6.13",
    "@types/node": "^20",
    "@types/react": "^19",
    "@types/react-dom": "^19",
    "@types/uuid": "^10",
    "electron": "^35.0.0",
    "electron-vite": "^3.0.0",
    "tailwindcss": "^4",
    "typescript": "^5",
    "vite": "^6",
    "vitest": "^3"
  }
}
```

**Step 3: Create electron.vite.config.ts**

```typescript
import { defineConfig, externalizeDepsPlugin } from 'electron-vite'
import tailwindcss from '@tailwindcss/vite'
import { resolve } from 'path'

export default defineConfig({
  main: {
    plugins: [externalizeDepsPlugin()],
    build: {
      rollupOptions: {
        input: {
          index: resolve(__dirname, 'electron/main.ts')
        }
      }
    }
  },
  preload: {
    plugins: [externalizeDepsPlugin()],
    build: {
      rollupOptions: {
        input: {
          index: resolve(__dirname, 'electron/preload.ts')
        }
      }
    }
  },
  renderer: {
    root: '.',
    build: {
      rollupOptions: {
        input: {
          index: resolve(__dirname, 'index.html')
        }
      }
    },
    plugins: [tailwindcss()]
  }
})
```

**Step 4: Create tsconfig files**

`tsconfig.json`:
```json
{
  "files": [],
  "references": [
    { "path": "./tsconfig.node.json" },
    { "path": "./tsconfig.web.json" }
  ]
}
```

`tsconfig.node.json`:
```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "outDir": "out",
    "rootDir": ".",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "resolveJsonModule": true,
    "declaration": true,
    "types": ["node"]
  },
  "include": ["electron/**/*.ts"]
}
```

`tsconfig.web.json`:
```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "jsx": "react-jsx",
    "outDir": "out",
    "rootDir": ".",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "resolveJsonModule": true,
    "types": ["node"]
  },
  "include": ["src/**/*.ts", "src/**/*.tsx"]
}
```

**Step 5: Create index.html**

```html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>Bot-HQ</title>
</head>
<body>
  <div id="root"></div>
  <script type="module" src="/src/main.tsx"></script>
</body>
</html>
```

**Step 6: Create minimal main process**

`electron/main.ts`:
```typescript
import { app, BrowserWindow } from 'electron'
import { join } from 'path'

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

app.whenReady().then(createWindow)

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') app.quit()
})
```

`electron/preload.ts`:
```typescript
import { contextBridge, ipcRenderer } from 'electron'

contextBridge.exposeInMainWorld('api', {
  send: (channel: string, data: unknown) => ipcRenderer.send(channel, data),
  on: (channel: string, callback: (...args: unknown[]) => void) => {
    ipcRenderer.on(channel, (_event, ...args) => callback(...args))
    return () => ipcRenderer.removeAllListeners(channel)
  },
  invoke: (channel: string, ...args: unknown[]) => ipcRenderer.invoke(channel, ...args)
})
```

**Step 7: Create minimal renderer**

`src/styles/globals.css`:
```css
@import "tailwindcss";

body {
  margin: 0;
  padding: 0;
  overflow: hidden;
  background: transparent;
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
}
```

`src/main.tsx`:
```tsx
import React from 'react'
import ReactDOM from 'react-dom/client'
import App from './App'
import './styles/globals.css'

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
)
```

`src/App.tsx`:
```tsx
export default function App() {
  return (
    <div className="h-screen w-screen rounded-2xl bg-zinc-900 text-white p-4 flex flex-col">
      <div className="flex items-center justify-between mb-4">
        <div className="flex items-center gap-2">
          <div className="w-3 h-3 rounded-full bg-green-500" />
          <span className="text-sm font-semibold">Bot-HQ</span>
        </div>
      </div>
      <div className="flex-1 flex items-center justify-center">
        <p className="text-zinc-500 text-sm">Ready</p>
      </div>
    </div>
  )
}
```

**Step 8: Install and run**

```bash
npm install
npm run dev
```

Expected: Electron window opens with floating dark panel showing "Bot-HQ" and "Ready".

**Step 9: Commit**

```bash
git add -A
git commit -m "feat: scaffold Electron app with Vite, React renderer, and floating window"
```

---

### Task 2: Global Hotkey + System Tray

**Files:**
- Create: `electron/hotkey.ts`
- Create: `electron/tray.ts`
- Modify: `electron/main.ts`

**Step 1: Create hotkey module**

`electron/hotkey.ts`:
```typescript
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
```

**Step 2: Create tray module**

`electron/tray.ts`:
```typescript
import { Tray, Menu, nativeImage, app, BrowserWindow } from 'electron'
import { join } from 'path'

let tray: Tray | null = null

export type TrayStatus = 'idle' | 'listening' | 'thinking' | 'executing' | 'speaking'

const STATUS_LABELS: Record<TrayStatus, string> = {
  idle: 'Bot-HQ — Idle',
  listening: 'Bot-HQ — Listening...',
  thinking: 'Bot-HQ — Thinking...',
  executing: 'Bot-HQ — Executing...',
  speaking: 'Bot-HQ — Speaking...'
}

export function createTray(window: BrowserWindow) {
  // Create a simple 16x16 tray icon
  const icon = nativeImage.createEmpty()
  tray = new Tray(icon)
  tray.setTitle('BH')
  updateTrayStatus('idle')

  tray.on('click', () => {
    window.isVisible() ? window.hide() : window.show()
  })
}

export function updateTrayStatus(status: TrayStatus) {
  if (!tray) return
  tray.setToolTip(STATUS_LABELS[status])
  tray.setTitle(status === 'idle' ? 'BH' : status === 'listening' ? '🎤' : status === 'thinking' ? '💭' : status === 'executing' ? '⚡' : '🔊')
}
```

**Step 3: Wire into main.ts**

Update `electron/main.ts` to import and use hotkey + tray modules. Add IPC handler so renderer can know about hotkey presses.

**Step 4: Test manually**

```bash
npm run dev
```

Expected: System tray icon appears. Cmd+Shift+Space triggers hotkey event. Window shows/hides on tray click.

**Step 5: Commit**

```bash
git add electron/hotkey.ts electron/tray.ts electron/main.ts
git commit -m "feat: add global hotkey (Cmd+Shift+Space) and system tray"
```

---

### Task 3: Audio Capture + Playback Pipeline

**Files:**
- Create: `electron/audio.ts`
- Modify: `electron/main.ts`
- Modify: `electron/preload.ts`
- Modify: `src/App.tsx`

**Step 1: Create audio module in main process**

`electron/audio.ts`:
```typescript
import { ipcMain, BrowserWindow } from 'electron'

// Audio is captured in the renderer (has access to MediaDevices API)
// and sent to main process via IPC for forwarding to Gemini.
// Main process sends back audio from Gemini to renderer for playback.

export function setupAudioIPC(window: BrowserWindow) {
  // Renderer sends captured PCM audio chunks
  ipcMain.on('audio:chunk', (_event, base64PCM: string) => {
    window.webContents.send('audio:to-gemini', base64PCM)
  })
}
```

**Step 2: Add audio capture to renderer**

The renderer uses `navigator.mediaDevices.getUserMedia` to capture mic audio, converts to 16kHz 16-bit mono PCM, and sends chunks via IPC.

Create `src/hooks/use-audio-capture.ts`:
```typescript
import { useCallback, useRef } from 'react'

export function useAudioCapture() {
  const streamRef = useRef<MediaStream | null>(null)
  const processorRef = useRef<ScriptProcessorNode | null>(null)
  const contextRef = useRef<AudioContext | null>(null)

  const start = useCallback(async () => {
    const stream = await navigator.mediaDevices.getUserMedia({
      audio: { sampleRate: 16000, channelCount: 1, echoCancellation: true }
    })
    streamRef.current = stream

    const context = new AudioContext({ sampleRate: 16000 })
    contextRef.current = context
    const source = context.createMediaStreamSource(stream)
    const processor = context.createScriptProcessor(4096, 1, 1)
    processorRef.current = processor

    processor.onaudioprocess = (e) => {
      const float32 = e.inputBuffer.getChannelData(0)
      // Convert Float32 to Int16 PCM
      const int16 = new Int16Array(float32.length)
      for (let i = 0; i < float32.length; i++) {
        const s = Math.max(-1, Math.min(1, float32[i]))
        int16[i] = s < 0 ? s * 0x8000 : s * 0x7fff
      }
      // Base64 encode and send to main process
      const bytes = new Uint8Array(int16.buffer)
      const base64 = btoa(String.fromCharCode(...bytes))
      window.api.send('audio:chunk', base64)
    }

    source.connect(processor)
    processor.connect(context.destination)
  }, [])

  const stop = useCallback(() => {
    processorRef.current?.disconnect()
    contextRef.current?.close()
    streamRef.current?.getTracks().forEach(t => t.stop())
    streamRef.current = null
  }, [])

  return { start, stop }
}
```

Create `src/hooks/use-audio-playback.ts`:
```typescript
import { useCallback, useRef } from 'react'

export function useAudioPlayback() {
  const contextRef = useRef<AudioContext | null>(null)
  const queueRef = useRef<AudioBuffer[]>([])
  const playingRef = useRef(false)

  const init = useCallback(() => {
    if (!contextRef.current) {
      contextRef.current = new AudioContext({ sampleRate: 24000 })
    }
  }, [])

  const enqueue = useCallback((base64PCM: string) => {
    init()
    const context = contextRef.current!
    const bytes = Uint8Array.from(atob(base64PCM), c => c.charCodeAt(0))
    const int16 = new Int16Array(bytes.buffer)
    const float32 = new Float32Array(int16.length)
    for (let i = 0; i < int16.length; i++) {
      float32[i] = int16[i] / 0x7fff
    }
    const buffer = context.createBuffer(1, float32.length, 24000)
    buffer.copyToChannel(float32, 0)
    queueRef.current.push(buffer)
    if (!playingRef.current) playNext()
  }, [])

  const playNext = useCallback(() => {
    const context = contextRef.current
    if (!context || queueRef.current.length === 0) {
      playingRef.current = false
      return
    }
    playingRef.current = true
    const buffer = queueRef.current.shift()!
    const source = context.createBufferSource()
    source.buffer = buffer
    source.connect(context.destination)
    source.onended = () => playNext()
    source.start()
  }, [])

  const stop = useCallback(() => {
    queueRef.current = []
    playingRef.current = false
  }, [])

  return { enqueue, stop }
}
```

**Step 3: Update preload to expose audio IPC**

Already covered by the generic `send`/`on` API in preload.

**Step 4: Test manually**

```bash
npm run dev
```

Expected: Pressing hotkey starts mic capture (browser permission prompt). Releasing stops. No audio goes anywhere yet (Gemini not connected).

**Step 5: Commit**

```bash
git add electron/audio.ts src/hooks/use-audio-capture.ts src/hooks/use-audio-playback.ts electron/main.ts electron/preload.ts src/App.tsx
git commit -m "feat: add audio capture (16kHz PCM) and playback (24kHz PCM) pipeline"
```

---

### Task 4: Gemini Live WebSocket Connection

**Files:**
- Create: `electron/gemini/client.ts`
- Create: `electron/gemini/session.ts`
- Create: `electron/gemini/types.ts`
- Modify: `electron/main.ts`

**Step 1: Create Gemini types**

`electron/gemini/types.ts`:
```typescript
export interface FunctionDeclaration {
  name: string
  description: string
  parameters: {
    type: string
    properties: Record<string, { type: string; description: string }>
    required?: string[]
  }
}

export interface FunctionCall {
  id: string
  name: string
  args: Record<string, unknown>
}

export interface ToolCallMessage {
  toolCall: {
    functionCalls: FunctionCall[]
  }
}

export interface ServerContentMessage {
  serverContent: {
    modelTurn?: {
      parts: Array<{
        text?: string
        inlineData?: { mimeType: string; data: string }
      }>
    }
    turnComplete?: boolean
    interrupted?: boolean
    inputTranscription?: { text: string }
    outputTranscription?: { text: string }
  }
}

export type GeminiMessage =
  | { setupComplete: object }
  | ToolCallMessage
  | ServerContentMessage
  | { toolCallCancellation: { ids: string[] } }
  | { goAway: { timeLeft: string } }

export type AgentState = 'idle' | 'listening' | 'thinking' | 'executing' | 'speaking'
```

**Step 2: Create Gemini client**

`electron/gemini/client.ts`:
```typescript
import { GoogleGenAI, Modality } from '@google/genai'
import { FunctionDeclaration, FunctionCall } from './types'
import { EventEmitter } from 'events'

export interface GeminiClientConfig {
  apiKey: string
  systemInstruction: string
  tools: FunctionDeclaration[]
}

export class GeminiClient extends EventEmitter {
  private ai: GoogleGenAI
  private session: any = null
  private config: GeminiClientConfig

  constructor(config: GeminiClientConfig) {
    super()
    this.config = config
    this.ai = new GoogleGenAI({ apiKey: config.apiKey })
  }

  async connect() {
    const toolDeclarations = this.config.tools.map(t => ({
      name: t.name,
      description: t.description,
      parameters: t.parameters
    }))

    this.session = await this.ai.live.connect({
      model: 'gemini-3.1-flash-live-preview',
      config: {
        responseModalities: [Modality.AUDIO],
        systemInstruction: {
          parts: [{ text: this.config.systemInstruction }]
        },
        tools: toolDeclarations.length > 0 ? [{ functionDeclarations: toolDeclarations }] : undefined,
        speechConfig: {
          voiceConfig: {
            prebuiltVoiceConfig: { voiceName: 'Kore' }
          }
        },
        inputAudioTranscription: {},
        outputAudioTranscription: {}
      },
      callbacks: {
        onopen: () => this.emit('connected'),
        onmessage: (msg: any) => this.handleMessage(msg),
        onerror: (e: any) => this.emit('error', e),
        onclose: (e: any) => this.emit('disconnected', e)
      }
    })

    return this.session
  }

  private handleMessage(msg: any) {
    // Tool calls
    if (msg.toolCall) {
      this.emit('tool-call', msg.toolCall.functionCalls as FunctionCall[])
      return
    }

    // Server content (audio, text, transcriptions)
    if (msg.serverContent) {
      const sc = msg.serverContent

      if (sc.modelTurn?.parts) {
        for (const part of sc.modelTurn.parts) {
          if (part.inlineData?.data) {
            this.emit('audio', part.inlineData.data)
          }
          if (part.text) {
            this.emit('text', part.text)
          }
        }
      }

      if (sc.inputTranscription?.text) {
        this.emit('user-transcript', sc.inputTranscription.text)
      }

      if (sc.outputTranscription?.text) {
        this.emit('assistant-transcript', sc.outputTranscription.text)
      }

      if (sc.turnComplete) {
        this.emit('turn-complete')
      }

      if (sc.interrupted) {
        this.emit('interrupted')
      }
    }

    if (msg.toolCallCancellation) {
      this.emit('tool-call-cancel', msg.toolCallCancellation.ids)
    }
  }

  sendAudio(base64PCM: string) {
    if (!this.session) return
    this.session.sendRealtimeInput({
      audio: {
        data: base64PCM,
        mimeType: 'audio/pcm;rate=16000'
      }
    })
  }

  sendToolResponse(responses: Array<{ id: string; name: string; response: unknown }>) {
    if (!this.session) return
    this.session.sendToolResponse({
      functionResponses: responses.map(r => ({
        id: r.id,
        name: r.name,
        response: { result: r.response }
      }))
    })
  }

  updateSystemInstruction(text: string) {
    this.config.systemInstruction = text
    // Note: requires reconnecting the session to update system instruction
  }

  async disconnect() {
    if (this.session) {
      this.session.close()
      this.session = null
    }
  }
}
```

**Step 3: Create session manager**

`electron/gemini/session.ts`:
```typescript
import { GeminiClient, GeminiClientConfig } from './client'
import { FunctionCall } from './types'
import { BrowserWindow } from 'electron'

export class GeminiSession {
  private client: GeminiClient
  private window: BrowserWindow

  constructor(config: GeminiClientConfig, window: BrowserWindow) {
    this.client = new GeminiClient(config)
    this.window = window
    this.setupListeners()
  }

  private setupListeners() {
    this.client.on('connected', () => {
      this.window.webContents.send('gemini:state', 'idle')
    })

    this.client.on('audio', (base64: string) => {
      this.window.webContents.send('gemini:audio', base64)
      this.window.webContents.send('gemini:state', 'speaking')
    })

    this.client.on('text', (text: string) => {
      this.window.webContents.send('gemini:text', text)
    })

    this.client.on('user-transcript', (text: string) => {
      this.window.webContents.send('gemini:user-transcript', text)
    })

    this.client.on('assistant-transcript', (text: string) => {
      this.window.webContents.send('gemini:assistant-transcript', text)
    })

    this.client.on('tool-call', (calls: FunctionCall[]) => {
      this.window.webContents.send('gemini:state', 'executing')
      this.window.webContents.send('gemini:tool-calls', calls)
    })

    this.client.on('turn-complete', () => {
      this.window.webContents.send('gemini:state', 'idle')
      this.window.webContents.send('gemini:turn-complete')
    })

    this.client.on('interrupted', () => {
      this.window.webContents.send('gemini:interrupted')
    })

    this.client.on('error', (error: Error) => {
      this.window.webContents.send('gemini:error', error.message)
    })

    this.client.on('disconnected', () => {
      this.window.webContents.send('gemini:state', 'idle')
    })
  }

  async connect() {
    await this.client.connect()
  }

  sendAudio(base64PCM: string) {
    this.client.sendAudio(base64PCM)
  }

  sendToolResponse(responses: Array<{ id: string; name: string; response: unknown }>) {
    this.client.sendToolResponse(responses)
  }

  async disconnect() {
    await this.client.disconnect()
  }
}
```

**Step 4: Wire Gemini into main.ts**

Update `electron/main.ts`:
- Load `GEMINI_API_KEY` from `.env` (use `dotenv` or read manually)
- Create `GeminiSession` on app ready
- Route audio chunks from renderer → Gemini
- Route Gemini audio → renderer for playback
- Route hotkey press → set state to `listening`

**Step 5: Test end-to-end voice loop**

```bash
npm run dev
```

Expected: Press Cmd+Shift+Space → mic captures → audio streams to Gemini → Gemini responds with audio → plays through speakers. No tools yet, just conversation.

**Step 6: Commit**

```bash
git add electron/gemini/ electron/main.ts
git commit -m "feat: integrate Gemini Live API with WebSocket audio streaming"
```

---

### Task 5: Floating Window UI with State Display

**Files:**
- Create: `src/components/floating-window.tsx`
- Create: `src/components/status-indicator.tsx`
- Create: `src/components/transcript.tsx`
- Modify: `src/App.tsx`

**Step 1: Create status indicator component**

`src/components/status-indicator.tsx`:
```tsx
import { AgentState } from '../../electron/gemini/types'

const STATE_CONFIG: Record<string, { label: string; color: string; pulse: boolean }> = {
  idle: { label: 'Ready', color: 'bg-zinc-500', pulse: false },
  listening: { label: 'Listening...', color: 'bg-red-500', pulse: true },
  thinking: { label: 'Thinking...', color: 'bg-yellow-500', pulse: true },
  executing: { label: 'Executing...', color: 'bg-blue-500', pulse: true },
  speaking: { label: 'Speaking...', color: 'bg-green-500', pulse: true }
}

export function StatusIndicator({ state }: { state: string }) {
  const config = STATE_CONFIG[state] || STATE_CONFIG.idle
  return (
    <div className="flex items-center gap-2">
      <div className={`w-3 h-3 rounded-full ${config.color} ${config.pulse ? 'animate-pulse' : ''}`} />
      <span className="text-sm text-zinc-300">{config.label}</span>
    </div>
  )
}
```

**Step 2: Create transcript component**

`src/components/transcript.tsx`:
```tsx
interface Message {
  role: 'user' | 'assistant'
  text: string
}

export function Transcript({ messages }: { messages: Message[] }) {
  return (
    <div className="flex-1 overflow-y-auto space-y-2 px-1">
      {messages.slice(-10).map((msg, i) => (
        <div key={i} className={`text-sm ${msg.role === 'user' ? 'text-blue-400' : 'text-zinc-300'}`}>
          <span className="text-zinc-600 text-xs">{msg.role === 'user' ? 'You' : 'Bot-HQ'}:</span>{' '}
          {msg.text}
        </div>
      ))}
    </div>
  )
}
```

**Step 3: Create floating window layout**

`src/components/floating-window.tsx`:
```tsx
import { StatusIndicator } from './status-indicator'
import { Transcript } from './transcript'

interface FloatingWindowProps {
  state: string
  messages: Array<{ role: 'user' | 'assistant'; text: string }>
  focusedProject: string | null
}

export function FloatingWindow({ state, messages, focusedProject }: FloatingWindowProps) {
  return (
    <div className="h-screen w-screen rounded-2xl bg-zinc-900/95 backdrop-blur-sm text-white p-4 flex flex-col border border-zinc-800 select-none" style={{ WebkitAppRegion: 'drag' } as any}>
      {/* Header */}
      <div className="flex items-center justify-between mb-3">
        <div className="flex items-center gap-2">
          <span className="text-sm font-bold">Bot-HQ</span>
          {focusedProject && (
            <span className="text-xs bg-zinc-800 text-zinc-400 px-2 py-0.5 rounded-full">
              {focusedProject}
            </span>
          )}
        </div>
        <StatusIndicator state={state} />
      </div>

      {/* Transcript */}
      <Transcript messages={messages} />

      {/* Footer hint */}
      <div className="mt-3 text-center text-zinc-600 text-xs">
        ⌘⇧Space to talk
      </div>
    </div>
  )
}
```

**Step 4: Wire into App.tsx**

Update `src/App.tsx` to use the floating window, listen for IPC events from main process (state changes, transcripts, etc.), and display them.

**Step 5: Test**

```bash
npm run dev
```

Expected: Floating window shows state transitions as you talk to Gemini. Transcripts appear for both user and assistant.

**Step 6: Commit**

```bash
git add src/components/ src/App.tsx
git commit -m "feat: add floating window UI with state indicator and transcript display"
```

---

## Phase 2: Tool System

### Task 6: Tool Registry + Dispatcher

**Files:**
- Create: `electron/tools/types.ts`
- Create: `electron/tools/registry.ts`
- Create: `electron/tools/dispatcher.ts`

**Step 1: Create tool types**

`electron/tools/types.ts`:
```typescript
export interface ToolParameter {
  type: string
  description: string
  enum?: string[]
}

export interface ToolDefinition {
  name: string
  description: string
  parameters: {
    type: 'OBJECT'
    properties: Record<string, ToolParameter>
    required?: string[]
  }
  destructive: boolean
  execute: (args: Record<string, unknown>) => Promise<unknown>
}
```

**Step 2: Create tool registry**

`electron/tools/registry.ts`:
```typescript
import { ToolDefinition } from './types'

class ToolRegistry {
  private tools = new Map<string, ToolDefinition>()

  register(tool: ToolDefinition) {
    this.tools.set(tool.name, tool)
  }

  registerAll(tools: ToolDefinition[]) {
    tools.forEach(t => this.register(t))
  }

  get(name: string): ToolDefinition | undefined {
    return this.tools.get(name)
  }

  getAll(): ToolDefinition[] {
    return Array.from(this.tools.values())
  }

  getFunctionDeclarations() {
    return this.getAll().map(t => ({
      name: t.name,
      description: t.description,
      parameters: t.parameters
    }))
  }
}

export const toolRegistry = new ToolRegistry()
```

**Step 3: Create dispatcher with safety checks**

`electron/tools/dispatcher.ts`:
```typescript
import { toolRegistry } from './registry'
import { BrowserWindow } from 'electron'

interface DispatchResult {
  id: string
  name: string
  response: unknown
}

export class ToolDispatcher {
  private window: BrowserWindow
  private pendingConfirmations = new Map<string, { resolve: (approved: boolean) => void }>()

  constructor(window: BrowserWindow) {
    this.window = window
  }

  async dispatch(calls: Array<{ id: string; name: string; args: Record<string, unknown> }>): Promise<DispatchResult[]> {
    const results: DispatchResult[] = []

    for (const call of calls) {
      const tool = toolRegistry.get(call.name)
      if (!tool) {
        results.push({ id: call.id, name: call.name, response: { error: `Unknown tool: ${call.name}` } })
        continue
      }

      // Emit action to UI
      this.window.webContents.send('tool:executing', { name: call.name, args: call.args })

      if (tool.destructive) {
        const approved = await this.requestConfirmation(call.id, call.name, call.args)
        if (!approved) {
          results.push({ id: call.id, name: call.name, response: { error: 'User denied this action' } })
          continue
        }
      }

      try {
        const result = await tool.execute(call.args)
        this.window.webContents.send('tool:completed', { name: call.name, success: true })
        results.push({ id: call.id, name: call.name, response: result })
      } catch (err: any) {
        this.window.webContents.send('tool:completed', { name: call.name, success: false, error: err.message })
        results.push({ id: call.id, name: call.name, response: { error: err.message } })
      }
    }

    return results
  }

  private requestConfirmation(id: string, name: string, args: Record<string, unknown>): Promise<boolean> {
    return new Promise(resolve => {
      this.pendingConfirmations.set(id, { resolve })
      this.window.webContents.send('tool:confirm', { id, name, args })
    })
  }

  handleConfirmationResponse(id: string, approved: boolean) {
    const pending = this.pendingConfirmations.get(id)
    if (pending) {
      pending.resolve(approved)
      this.pendingConfirmations.delete(id)
    }
  }
}
```

**Step 4: Commit**

```bash
git add electron/tools/
git commit -m "feat: add tool registry and dispatcher with destructive action confirmation"
```

---

### Task 7: File Operation Tools

**Files:**
- Create: `electron/tools/files.ts`

**Step 1: Implement file tools**

`electron/tools/files.ts`:
```typescript
import { ToolDefinition } from './types'
import { readFile, writeFile, unlink, rename, copyFile, stat, readdir } from 'fs/promises'
import { join, resolve } from 'path'
import { execSync } from 'child_process'

export const fileTools: ToolDefinition[] = [
  {
    name: 'read_file',
    description: 'Read the contents of a file at the given path',
    parameters: {
      type: 'OBJECT',
      properties: {
        path: { type: 'STRING', description: 'Absolute or relative file path' }
      },
      required: ['path']
    },
    destructive: false,
    execute: async (args) => {
      const content = await readFile(resolve(args.path as string), 'utf-8')
      return { content: content.slice(0, 50000) } // limit output
    }
  },
  {
    name: 'write_file',
    description: 'Create or overwrite a file with the given content',
    parameters: {
      type: 'OBJECT',
      properties: {
        path: { type: 'STRING', description: 'File path' },
        content: { type: 'STRING', description: 'File content' }
      },
      required: ['path', 'content']
    },
    destructive: false,
    execute: async (args) => {
      await writeFile(resolve(args.path as string), args.content as string, 'utf-8')
      return { success: true }
    }
  },
  {
    name: 'delete_file',
    description: 'Delete a file at the given path',
    parameters: {
      type: 'OBJECT',
      properties: {
        path: { type: 'STRING', description: 'File path to delete' }
      },
      required: ['path']
    },
    destructive: true,
    execute: async (args) => {
      await unlink(resolve(args.path as string))
      return { success: true }
    }
  },
  {
    name: 'move_file',
    description: 'Move or rename a file',
    parameters: {
      type: 'OBJECT',
      properties: {
        from: { type: 'STRING', description: 'Source path' },
        to: { type: 'STRING', description: 'Destination path' }
      },
      required: ['from', 'to']
    },
    destructive: false,
    execute: async (args) => {
      await rename(resolve(args.from as string), resolve(args.to as string))
      return { success: true }
    }
  },
  {
    name: 'copy_file',
    description: 'Copy a file from source to destination',
    parameters: {
      type: 'OBJECT',
      properties: {
        from: { type: 'STRING', description: 'Source path' },
        to: { type: 'STRING', description: 'Destination path' }
      },
      required: ['from', 'to']
    },
    destructive: false,
    execute: async (args) => {
      await copyFile(resolve(args.from as string), resolve(args.to as string))
      return { success: true }
    }
  },
  {
    name: 'list_directory',
    description: 'List files and directories at the given path',
    parameters: {
      type: 'OBJECT',
      properties: {
        path: { type: 'STRING', description: 'Directory path' }
      },
      required: ['path']
    },
    destructive: false,
    execute: async (args) => {
      const entries = await readdir(resolve(args.path as string), { withFileTypes: true })
      return {
        entries: entries.map(e => ({
          name: e.name,
          type: e.isDirectory() ? 'directory' : 'file'
        }))
      }
    }
  },
  {
    name: 'search_files',
    description: 'Search for files matching a glob pattern',
    parameters: {
      type: 'OBJECT',
      properties: {
        pattern: { type: 'STRING', description: 'Glob pattern (e.g. "**/*.ts")' },
        directory: { type: 'STRING', description: 'Directory to search in' }
      },
      required: ['pattern', 'directory']
    },
    destructive: false,
    execute: async (args) => {
      const { globSync } = await import('glob')
      const matches = globSync(args.pattern as string, { cwd: resolve(args.directory as string) })
      return { files: matches.slice(0, 200) }
    }
  },
  {
    name: 'search_content',
    description: 'Search file contents for a pattern using ripgrep or grep',
    parameters: {
      type: 'OBJECT',
      properties: {
        pattern: { type: 'STRING', description: 'Search pattern (regex)' },
        directory: { type: 'STRING', description: 'Directory to search in' },
        file_glob: { type: 'STRING', description: 'Optional file glob filter (e.g. "*.ts")' }
      },
      required: ['pattern', 'directory']
    },
    destructive: false,
    execute: async (args) => {
      const dir = resolve(args.directory as string)
      const glob = args.file_glob ? `--glob '${args.file_glob}'` : ''
      try {
        const result = execSync(`rg --json -m 50 ${glob} '${args.pattern}' '${dir}'`, { encoding: 'utf-8', timeout: 10000 })
        const lines = result.trim().split('\n').filter(l => l.startsWith('{"type":"match"'))
        return { matches: lines.slice(0, 50).map(l => JSON.parse(l)) }
      } catch {
        return { matches: [] }
      }
    }
  },
  {
    name: 'file_info',
    description: 'Get metadata about a file (size, timestamps, permissions)',
    parameters: {
      type: 'OBJECT',
      properties: {
        path: { type: 'STRING', description: 'File path' }
      },
      required: ['path']
    },
    destructive: false,
    execute: async (args) => {
      const s = await stat(resolve(args.path as string))
      return {
        size: s.size,
        created: s.birthtime.toISOString(),
        modified: s.mtime.toISOString(),
        isDirectory: s.isDirectory(),
        permissions: s.mode.toString(8)
      }
    }
  },
  {
    name: 'edit_file',
    description: 'Find and replace text in a file',
    parameters: {
      type: 'OBJECT',
      properties: {
        path: { type: 'STRING', description: 'File path' },
        old_text: { type: 'STRING', description: 'Text to find' },
        new_text: { type: 'STRING', description: 'Text to replace with' }
      },
      required: ['path', 'old_text', 'new_text']
    },
    destructive: false,
    execute: async (args) => {
      const filePath = resolve(args.path as string)
      const content = await readFile(filePath, 'utf-8')
      if (!content.includes(args.old_text as string)) {
        return { error: 'old_text not found in file' }
      }
      const updated = content.replace(args.old_text as string, args.new_text as string)
      await writeFile(filePath, updated, 'utf-8')
      return { success: true }
    }
  }
]
```

**Step 2: Register in tool registry**

Add to `electron/main.ts`:
```typescript
import { fileTools } from './tools/files'
toolRegistry.registerAll(fileTools)
```

**Step 3: Commit**

```bash
git add electron/tools/files.ts electron/main.ts
git commit -m "feat: add file operation tools (read, write, edit, delete, move, copy, search)"
```

---

### Task 8: Shell + System Tools

**Files:**
- Create: `electron/tools/shell.ts`
- Create: `electron/safety/patterns.ts`
- Create: `electron/safety/checker.ts`

**Step 1: Create destructive patterns**

`electron/safety/patterns.ts`:
```typescript
export const DESTRUCTIVE_PATTERNS = [
  /^rm\s/,
  /^sudo\s/,
  /^kill\s/,
  /^pkill\s/,
  /^shutdown/,
  /^reboot/,
  /^git\s+push.*--force/,
  /^git\s+reset\s+--hard/,
  /^git\s+clean\s+-f/,
  /^chmod\s/,
  /^chown\s/,
  /drop\s+table/i,
  /delete\s+from/i,
  /truncate\s/i,
]

export function isDestructiveCommand(command: string): boolean {
  return DESTRUCTIVE_PATTERNS.some(p => p.test(command.trim()))
}
```

**Step 2: Create safety checker**

`electron/safety/checker.ts`:
```typescript
import { isDestructiveCommand } from './patterns'

export function checkSafety(toolName: string, args: Record<string, unknown>): boolean {
  if (toolName === 'run_command') {
    return isDestructiveCommand(args.command as string)
  }
  return false // tool's own destructive flag handles other cases
}
```

**Step 3: Create shell tools**

`electron/tools/shell.ts`:
```typescript
import { ToolDefinition } from './types'
import { exec, execSync } from 'child_process'
import { isDestructiveCommand } from '../safety/patterns'
import { promisify } from 'util'

const execAsync = promisify(exec)

export const shellTools: ToolDefinition[] = [
  {
    name: 'run_command',
    description: 'Execute a shell command and return stdout/stderr. Use for any terminal command.',
    parameters: {
      type: 'OBJECT',
      properties: {
        command: { type: 'STRING', description: 'Shell command to execute' },
        cwd: { type: 'STRING', description: 'Working directory (optional)' },
        timeout_ms: { type: 'NUMBER', description: 'Timeout in milliseconds (default 30000)' }
      },
      required: ['command']
    },
    get destructive() { return false }, // Dynamic: checked at dispatch time via safety checker
    execute: async (args) => {
      const timeout = (args.timeout_ms as number) || 30000
      try {
        const { stdout, stderr } = await execAsync(args.command as string, {
          cwd: args.cwd as string || process.env.HOME,
          timeout,
          maxBuffer: 1024 * 1024,
          shell: '/bin/zsh'
        })
        return { stdout: stdout.slice(0, 20000), stderr: stderr.slice(0, 5000) }
      } catch (err: any) {
        return { error: err.message, stdout: err.stdout?.slice(0, 10000), stderr: err.stderr?.slice(0, 5000) }
      }
    }
  },
  {
    name: 'open_app',
    description: 'Open an application by name (macOS)',
    parameters: {
      type: 'OBJECT',
      properties: {
        name: { type: 'STRING', description: 'Application name (e.g. "Visual Studio Code", "Finder", "Safari")' }
      },
      required: ['name']
    },
    destructive: false,
    execute: async (args) => {
      execSync(`open -a "${args.name}"`)
      return { success: true, opened: args.name }
    }
  },
  {
    name: 'kill_process',
    description: 'Kill a process by name or PID',
    parameters: {
      type: 'OBJECT',
      properties: {
        target: { type: 'STRING', description: 'Process name or PID' }
      },
      required: ['target']
    },
    destructive: true,
    execute: async (args) => {
      const target = args.target as string
      const cmd = /^\d+$/.test(target) ? `kill ${target}` : `pkill -f "${target}"`
      execSync(cmd)
      return { success: true, killed: target }
    }
  },
  {
    name: 'system_info',
    description: 'Get system information: CPU, memory, disk usage',
    parameters: { type: 'OBJECT', properties: {}, required: [] },
    destructive: false,
    execute: async () => {
      const cpu = execSync("sysctl -n machdep.cpu.brand_string", { encoding: 'utf-8' }).trim()
      const mem = execSync("vm_stat | head -5", { encoding: 'utf-8' }).trim()
      const disk = execSync("df -h / | tail -1", { encoding: 'utf-8' }).trim()
      const uptime = execSync("uptime", { encoding: 'utf-8' }).trim()
      return { cpu, memory: mem, disk, uptime }
    }
  },
  {
    name: 'list_processes',
    description: 'List running processes, optionally filtered by name',
    parameters: {
      type: 'OBJECT',
      properties: {
        filter: { type: 'STRING', description: 'Optional name filter' }
      }
    },
    destructive: false,
    execute: async (args) => {
      const filter = args.filter ? `| grep -i "${args.filter}"` : '| head -30'
      const result = execSync(`ps aux ${filter}`, { encoding: 'utf-8' })
      return { processes: result.trim() }
    }
  }
]
```

**Step 4: Update dispatcher to check dynamic destructive status for run_command**

In `electron/tools/dispatcher.ts`, before the `tool.destructive` check, add:
```typescript
import { isDestructiveCommand } from '../safety/patterns'

// In dispatch():
const isDestructive = tool.destructive ||
  (call.name === 'run_command' && isDestructiveCommand(call.args.command as string))
```

**Step 5: Commit**

```bash
git add electron/tools/shell.ts electron/safety/
git commit -m "feat: add shell/system tools with destructive command detection"
```

---

### Task 9: Git Tools

**Files:**
- Create: `electron/tools/git.ts`

**Step 1: Implement git tools using simple-git**

`electron/tools/git.ts`:
```typescript
import { ToolDefinition } from './types'
import simpleGit from 'simple-git'

function getGit(cwd?: string) {
  return simpleGit(cwd || process.cwd())
}

export const gitTools: ToolDefinition[] = [
  {
    name: 'git_status',
    description: 'Show git working tree status',
    parameters: {
      type: 'OBJECT',
      properties: {
        cwd: { type: 'STRING', description: 'Repository directory' }
      },
      required: ['cwd']
    },
    destructive: false,
    execute: async (args) => {
      const status = await getGit(args.cwd as string).status()
      return status
    }
  },
  {
    name: 'git_diff',
    description: 'Show git diff (staged and unstaged)',
    parameters: {
      type: 'OBJECT',
      properties: {
        cwd: { type: 'STRING', description: 'Repository directory' },
        staged: { type: 'BOOLEAN', description: 'Show staged changes only' }
      },
      required: ['cwd']
    },
    destructive: false,
    execute: async (args) => {
      const git = getGit(args.cwd as string)
      const diff = args.staged ? await git.diff(['--cached']) : await git.diff()
      return { diff: diff.slice(0, 30000) }
    }
  },
  {
    name: 'git_log',
    description: 'Show recent commit history',
    parameters: {
      type: 'OBJECT',
      properties: {
        cwd: { type: 'STRING', description: 'Repository directory' },
        count: { type: 'NUMBER', description: 'Number of commits (default 10)' }
      },
      required: ['cwd']
    },
    destructive: false,
    execute: async (args) => {
      const log = await getGit(args.cwd as string).log({ maxCount: (args.count as number) || 10 })
      return { commits: log.all }
    }
  },
  {
    name: 'git_commit',
    description: 'Stage files and create a commit',
    parameters: {
      type: 'OBJECT',
      properties: {
        cwd: { type: 'STRING', description: 'Repository directory' },
        message: { type: 'STRING', description: 'Commit message' },
        files: { type: 'STRING', description: 'Files to stage (space-separated, or "." for all)' }
      },
      required: ['cwd', 'message']
    },
    destructive: false,
    execute: async (args) => {
      const git = getGit(args.cwd as string)
      const files = (args.files as string) || '.'
      await git.add(files.split(' '))
      const result = await git.commit(args.message as string)
      return result
    }
  },
  {
    name: 'git_push',
    description: 'Push commits to remote',
    parameters: {
      type: 'OBJECT',
      properties: {
        cwd: { type: 'STRING', description: 'Repository directory' },
        remote: { type: 'STRING', description: 'Remote name (default: origin)' },
        branch: { type: 'STRING', description: 'Branch name (optional)' }
      },
      required: ['cwd']
    },
    destructive: true,
    execute: async (args) => {
      const git = getGit(args.cwd as string)
      const result = await git.push(args.remote as string || 'origin', args.branch as string)
      return result
    }
  },
  {
    name: 'git_pull',
    description: 'Pull changes from remote',
    parameters: {
      type: 'OBJECT',
      properties: {
        cwd: { type: 'STRING', description: 'Repository directory' }
      },
      required: ['cwd']
    },
    destructive: false,
    execute: async (args) => {
      const result = await getGit(args.cwd as string).pull()
      return result
    }
  },
  {
    name: 'git_branch',
    description: 'List, create, or switch branches',
    parameters: {
      type: 'OBJECT',
      properties: {
        cwd: { type: 'STRING', description: 'Repository directory' },
        action: { type: 'STRING', description: '"list", "create", or "switch"', enum: ['list', 'create', 'switch'] },
        name: { type: 'STRING', description: 'Branch name (for create/switch)' }
      },
      required: ['cwd', 'action']
    },
    destructive: false,
    execute: async (args) => {
      const git = getGit(args.cwd as string)
      switch (args.action) {
        case 'list': return await git.branch()
        case 'create': await git.checkoutLocalBranch(args.name as string); return { created: args.name }
        case 'switch': await git.checkout(args.name as string); return { switched: args.name }
        default: return { error: 'Invalid action' }
      }
    }
  },
  {
    name: 'git_stash',
    description: 'Stash or pop stashed changes',
    parameters: {
      type: 'OBJECT',
      properties: {
        cwd: { type: 'STRING', description: 'Repository directory' },
        action: { type: 'STRING', description: '"push", "pop", or "list"', enum: ['push', 'pop', 'list'] }
      },
      required: ['cwd', 'action']
    },
    destructive: false,
    execute: async (args) => {
      const git = getGit(args.cwd as string)
      switch (args.action) {
        case 'push': return await git.stash()
        case 'pop': return await git.stash(['pop'])
        case 'list': return await git.stashList()
        default: return { error: 'Invalid action' }
      }
    }
  }
]
```

**Step 2: Register and commit**

```bash
git add electron/tools/git.ts
git commit -m "feat: add git tools (status, diff, log, commit, push, pull, branch, stash)"
```

---

### Task 10: Screen + Browser Tools

**Files:**
- Create: `electron/tools/screen.ts`
- Create: `electron/tools/browser.ts`

**Step 1: Create screenshot tools**

`electron/tools/screen.ts`:
```typescript
import { ToolDefinition } from './types'
import { desktopCapturer, screen as electronScreen } from 'electron'
import { writeFile } from 'fs/promises'
import { join } from 'path'
import { tmpdir } from 'os'
import { v4 as uuid } from 'uuid'

export const screenTools: ToolDefinition[] = [
  {
    name: 'take_screenshot',
    description: 'Capture a screenshot of the entire screen. Returns the image as base64.',
    parameters: { type: 'OBJECT', properties: {}, required: [] },
    destructive: false,
    execute: async () => {
      const sources = await desktopCapturer.getSources({
        types: ['screen'],
        thumbnailSize: electronScreen.getPrimaryDisplay().workAreaSize
      })
      const primarySource = sources[0]
      if (!primarySource) return { error: 'No screen source found' }
      const image = primarySource.thumbnail
      const base64 = image.toPNG().toString('base64')
      // Save to temp file for reference
      const path = join(tmpdir(), `screenshot-${uuid()}.png`)
      await writeFile(path, image.toPNG())
      return { base64, path, width: image.getSize().width, height: image.getSize().height }
    }
  },
  {
    name: 'read_screen',
    description: 'Take a screenshot and describe what is currently visible on screen. Returns screenshot base64 for Gemini to analyze.',
    parameters: {
      type: 'OBJECT',
      properties: {
        question: { type: 'STRING', description: 'What to look for on screen (optional)' }
      }
    },
    destructive: false,
    execute: async () => {
      const sources = await desktopCapturer.getSources({
        types: ['screen'],
        thumbnailSize: electronScreen.getPrimaryDisplay().workAreaSize
      })
      const primarySource = sources[0]
      if (!primarySource) return { error: 'No screen source found' }
      const image = primarySource.thumbnail
      return {
        base64: image.toPNG().toString('base64'),
        width: image.getSize().width,
        height: image.getSize().height,
        mimeType: 'image/png'
      }
    }
  }
]
```

**Step 2: Create browser tools**

`electron/tools/browser.ts`:
```typescript
import { ToolDefinition } from './types'
import { execSync } from 'child_process'

export const browserTools: ToolDefinition[] = [
  {
    name: 'web_search',
    description: 'Search the web using a query. Returns search results.',
    parameters: {
      type: 'OBJECT',
      properties: {
        query: { type: 'STRING', description: 'Search query' }
      },
      required: ['query']
    },
    destructive: false,
    execute: async (args) => {
      // Use Google Custom Search API or fallback to opening browser
      const query = encodeURIComponent(args.query as string)
      execSync(`open "https://www.google.com/search?q=${query}"`)
      return { success: true, message: `Opened search for: ${args.query}` }
    }
  },
  {
    name: 'open_url',
    description: 'Open a URL in the default browser',
    parameters: {
      type: 'OBJECT',
      properties: {
        url: { type: 'STRING', description: 'URL to open' }
      },
      required: ['url']
    },
    destructive: false,
    execute: async (args) => {
      execSync(`open "${args.url}"`)
      return { success: true, opened: args.url }
    }
  },
  {
    name: 'fetch_page',
    description: 'Fetch a web page and extract its text content',
    parameters: {
      type: 'OBJECT',
      properties: {
        url: { type: 'STRING', description: 'URL to fetch' }
      },
      required: ['url']
    },
    destructive: false,
    execute: async (args) => {
      const response = await fetch(args.url as string)
      const html = await response.text()
      // Strip HTML tags for plain text
      const text = html.replace(/<script[^>]*>[\s\S]*?<\/script>/gi, '')
        .replace(/<style[^>]*>[\s\S]*?<\/style>/gi, '')
        .replace(/<[^>]+>/g, ' ')
        .replace(/\s+/g, ' ')
        .trim()
      return { text: text.slice(0, 30000), url: args.url }
    }
  }
]
```

**Step 3: Commit**

```bash
git add electron/tools/screen.ts electron/tools/browser.ts
git commit -m "feat: add screen capture and browser tools"
```

---

## Phase 3: Memory & Context

### Task 11: SQLite Schema + Drizzle Setup

**Files:**
- Create: `electron/memory/schema.ts`
- Create: `electron/memory/db.ts`
- Create: `drizzle.config.ts`

**Step 1: Create schema**

`electron/memory/schema.ts`:
```typescript
import { sqliteTable, text, integer } from 'drizzle-orm/sqlite-core'

export const conversations = sqliteTable('conversations', {
  id: text('id').primaryKey(),
  startedAt: text('started_at').notNull(),
  endedAt: text('ended_at'),
  projectPath: text('project_path'),
  summary: text('summary')
})

export const messages = sqliteTable('messages', {
  id: text('id').primaryKey(),
  conversationId: text('conversation_id').notNull().references(() => conversations.id),
  role: text('role').notNull(), // 'user' | 'assistant' | 'tool_call' | 'tool_result'
  content: text('content').notNull(),
  timestamp: text('timestamp').notNull(),
  tokenCount: integer('token_count')
})

export const memories = sqliteTable('memories', {
  id: text('id').primaryKey(),
  category: text('category').notNull(), // 'preference' | 'fact' | 'decision' | 'person' | 'project'
  content: text('content').notNull(),
  sourceConversationId: text('source_conversation_id'),
  createdAt: text('created_at').notNull(),
  lastAccessedAt: text('last_accessed_at'),
  accessCount: integer('access_count').default(0)
})

export const projects = sqliteTable('projects', {
  id: text('id').primaryKey(),
  name: text('name').notNull(),
  path: text('path').notNull().unique(),
  description: text('description'),
  lastFocusedAt: text('last_focused_at'),
  fileTreeSnapshot: text('file_tree_snapshot'), // JSON
  keyFiles: text('key_files'), // JSON
  conventions: text('conventions'),
  createdAt: text('created_at').notNull()
})

export const toolExecutions = sqliteTable('tool_executions', {
  id: text('id').primaryKey(),
  conversationId: text('conversation_id').references(() => conversations.id),
  toolName: text('tool_name').notNull(),
  parameters: text('parameters').notNull(), // JSON
  result: text('result'), // JSON
  success: integer('success').notNull(),
  durationMs: integer('duration_ms'),
  executedAt: text('executed_at').notNull()
})

export const claudeSessions = sqliteTable('claude_sessions', {
  id: text('id').primaryKey(),
  projectPath: text('project_path'),
  pid: integer('pid'),
  tmuxTarget: text('tmux_target'),
  mode: text('mode').notNull(), // 'oneshot' | 'managed' | 'attached'
  status: text('status').notNull(), // 'running' | 'completed' | 'failed' | 'stopped'
  lastOutput: text('last_output'),
  lastCheckedAt: text('last_checked_at'),
  startedAt: text('started_at').notNull(),
  endedAt: text('ended_at')
})
```

**Step 2: Create database initialization**

`electron/memory/db.ts`:
```typescript
import Database from 'better-sqlite3'
import { drizzle } from 'drizzle-orm/better-sqlite3'
import { migrate } from 'drizzle-orm/better-sqlite3/migrator'
import { app } from 'electron'
import { join } from 'path'
import { mkdirSync } from 'fs'
import * as schema from './schema'

let db: ReturnType<typeof drizzle> | null = null

export function getDb() {
  if (db) return db

  const dataDir = join(app.getPath('userData'), 'data')
  mkdirSync(dataDir, { recursive: true })

  const sqlite = new Database(join(dataDir, 'bot-hq.db'))
  sqlite.pragma('journal_mode = WAL')
  sqlite.pragma('foreign_keys = ON')

  db = drizzle(sqlite, { schema })

  // Run migrations
  migrate(db, { migrationsFolder: join(__dirname, '../../drizzle') })

  return db
}

export { schema }
```

**Step 3: Create drizzle config**

`drizzle.config.ts`:
```typescript
import { defineConfig } from 'drizzle-kit'

export default defineConfig({
  schema: './electron/memory/schema.ts',
  out: './drizzle',
  dialect: 'sqlite'
})
```

**Step 4: Generate migrations and commit**

```bash
npx drizzle-kit generate
git add electron/memory/ drizzle.config.ts drizzle/
git commit -m "feat: add SQLite schema with Drizzle ORM (conversations, memories, projects, sessions)"
```

---

### Task 12: Memory Tools (Remember/Recall/Forget)

**Files:**
- Create: `electron/tools/memory.ts`

**Step 1: Implement memory tools**

`electron/tools/memory.ts`:
```typescript
import { ToolDefinition } from './types'
import { getDb, schema } from '../memory/db'
import { eq, like, desc, sql } from 'drizzle-orm'
import { v4 as uuid } from 'uuid'

export const memoryTools: ToolDefinition[] = [
  {
    name: 'remember',
    description: 'Store a fact, preference, or decision in long-term memory for future recall',
    parameters: {
      type: 'OBJECT',
      properties: {
        content: { type: 'STRING', description: 'What to remember' },
        category: { type: 'STRING', description: 'Category: preference, fact, decision, person, or project', enum: ['preference', 'fact', 'decision', 'person', 'project'] }
      },
      required: ['content', 'category']
    },
    destructive: false,
    execute: async (args) => {
      const db = getDb()
      const id = uuid()
      await db.insert(schema.memories).values({
        id,
        content: args.content as string,
        category: args.category as string,
        createdAt: new Date().toISOString()
      })
      return { success: true, id }
    }
  },
  {
    name: 'recall',
    description: 'Search long-term memory for stored facts, preferences, or decisions',
    parameters: {
      type: 'OBJECT',
      properties: {
        query: { type: 'STRING', description: 'Search query' },
        category: { type: 'STRING', description: 'Optional category filter' }
      },
      required: ['query']
    },
    destructive: false,
    execute: async (args) => {
      const db = getDb()
      const query = `%${args.query}%`
      let results
      if (args.category) {
        results = await db.select().from(schema.memories)
          .where(sql`${schema.memories.content} LIKE ${query} AND ${schema.memories.category} = ${args.category}`)
          .orderBy(desc(schema.memories.lastAccessedAt))
          .limit(20)
      } else {
        results = await db.select().from(schema.memories)
          .where(like(schema.memories.content, query))
          .orderBy(desc(schema.memories.lastAccessedAt))
          .limit(20)
      }
      // Update access timestamps
      for (const r of results) {
        await db.update(schema.memories)
          .set({ lastAccessedAt: new Date().toISOString(), accessCount: (r.accessCount || 0) + 1 })
          .where(eq(schema.memories.id, r.id))
      }
      return { memories: results }
    }
  },
  {
    name: 'forget',
    description: 'Remove a memory by ID',
    parameters: {
      type: 'OBJECT',
      properties: {
        id: { type: 'STRING', description: 'Memory ID to remove' }
      },
      required: ['id']
    },
    destructive: true,
    execute: async (args) => {
      const db = getDb()
      await db.delete(schema.memories).where(eq(schema.memories.id, args.id as string))
      return { success: true }
    }
  }
]
```

**Step 2: Commit**

```bash
git add electron/tools/memory.ts
git commit -m "feat: add memory tools (remember, recall, forget) backed by SQLite"
```

---

### Task 13: Context Assembly + System Instruction

**Files:**
- Create: `electron/memory/context.ts`

**Step 1: Create context builder**

`electron/memory/context.ts`:
```typescript
import { getDb, schema } from './db'
import { desc, eq } from 'drizzle-orm'

const BASE_SYSTEM_PROMPT = `You are Bot-HQ, a voice-controlled computer agent running on macOS. You help the user by executing tools on their machine.

You can:
- Read, write, edit, search, and manage files
- Run shell commands and control system processes
- Manage git repositories
- Take screenshots and understand what's on screen
- Open apps and URLs
- Remember facts and preferences for future conversations
- Focus on specific projects for context-aware assistance
- Start and manage Claude Code sessions for complex coding tasks

Guidelines:
- Be concise in voice responses — the user is listening, not reading
- For destructive actions (delete, kill, force push), always explain what you're about to do
- When focused on a project, scope file and git operations to that project by default
- Use the remember tool to store important facts the user tells you
- Use the recall tool to check memory before asking the user something they may have told you before
`

export async function buildSystemInstruction(): Promise<string> {
  const db = getDb()
  const parts: string[] = [BASE_SYSTEM_PROMPT]

  // Add long-term memories (most recently accessed first)
  const memories = await db.select().from(schema.memories)
    .orderBy(desc(schema.memories.lastAccessedAt))
    .limit(30)

  if (memories.length > 0) {
    parts.push('\n## Your Memories')
    for (const m of memories) {
      parts.push(`- [${m.category}] ${m.content}`)
    }
  }

  // Add focused project context
  const focusedProject = await db.select().from(schema.projects)
    .orderBy(desc(schema.projects.lastFocusedAt))
    .limit(1)

  if (focusedProject.length > 0 && focusedProject[0].lastFocusedAt) {
    const p = focusedProject[0]
    parts.push(`\n## Currently Focused Project: ${p.name}`)
    parts.push(`Path: ${p.path}`)
    if (p.description) parts.push(`Description: ${p.description}`)
    if (p.conventions) parts.push(`Conventions: ${p.conventions}`)
    if (p.keyFiles) {
      try {
        const keyFiles = JSON.parse(p.keyFiles)
        parts.push('\nKey files:')
        for (const [name, content] of Object.entries(keyFiles)) {
          parts.push(`\n### ${name}\n\`\`\`\n${(content as string).slice(0, 2000)}\n\`\`\``)
        }
      } catch {}
    }
  }

  // Add last conversation summary
  const lastConvo = await db.select().from(schema.conversations)
    .orderBy(desc(schema.conversations.startedAt))
    .limit(1)

  if (lastConvo.length > 0 && lastConvo[0].summary) {
    parts.push(`\n## Last Conversation Summary\n${lastConvo[0].summary}`)
  }

  return parts.join('\n')
}
```

**Step 2: Commit**

```bash
git add electron/memory/context.ts
git commit -m "feat: add context assembly for Gemini system instruction (memories, project, history)"
```

---

## Phase 4: Project Focus

### Task 14: Project Scanner + Focus Tools

**Files:**
- Create: `electron/memory/project-scanner.ts`
- Create: `electron/tools/project.ts`

**Step 1: Create project scanner**

`electron/memory/project-scanner.ts`:
```typescript
import { readdir, readFile, stat } from 'fs/promises'
import { join, basename } from 'path'

const KEY_FILE_NAMES = [
  'package.json', 'README.md', 'CLAUDE.md', 'tsconfig.json',
  'Cargo.toml', 'go.mod', 'pyproject.toml', 'Makefile',
  '.env.example', 'docker-compose.yml', 'Dockerfile'
]

const IGNORE_DIRS = new Set([
  'node_modules', '.git', '.next', 'dist', 'out', 'build',
  '__pycache__', '.venv', 'vendor', 'target', '.turbo'
])

export async function scanProject(projectPath: string) {
  const tree = await buildFileTree(projectPath, 3)
  const keyFiles = await readKeyFiles(projectPath)
  const name = basename(projectPath)

  return { name, tree, keyFiles }
}

async function buildFileTree(dir: string, maxDepth: number, depth = 0): Promise<string> {
  if (depth >= maxDepth) return ''
  const entries = await readdir(dir, { withFileTypes: true })
  const lines: string[] = []
  const indent = '  '.repeat(depth)

  for (const entry of entries.sort((a, b) => a.name.localeCompare(b.name))) {
    if (IGNORE_DIRS.has(entry.name) || entry.name.startsWith('.')) continue
    if (entry.isDirectory()) {
      lines.push(`${indent}${entry.name}/`)
      lines.push(await buildFileTree(join(dir, entry.name), maxDepth, depth + 1))
    } else {
      lines.push(`${indent}${entry.name}`)
    }
  }

  return lines.filter(Boolean).join('\n')
}

async function readKeyFiles(dir: string): Promise<Record<string, string>> {
  const result: Record<string, string> = {}
  for (const name of KEY_FILE_NAMES) {
    try {
      const content = await readFile(join(dir, name), 'utf-8')
      result[name] = content.slice(0, 3000)
    } catch {}
  }
  return result
}
```

**Step 2: Create project focus tools**

`electron/tools/project.ts`:
```typescript
import { ToolDefinition } from './types'
import { getDb, schema } from '../memory/db'
import { eq, desc } from 'drizzle-orm'
import { scanProject } from '../memory/project-scanner'
import { v4 as uuid } from 'uuid'

export const projectTools: ToolDefinition[] = [
  {
    name: 'focus_project',
    description: 'Focus on a project directory. Scans the project and loads context so all subsequent actions are project-aware.',
    parameters: {
      type: 'OBJECT',
      properties: {
        path: { type: 'STRING', description: 'Absolute path to the project directory' }
      },
      required: ['path']
    },
    destructive: false,
    execute: async (args) => {
      const projectPath = args.path as string
      const { name, tree, keyFiles } = await scanProject(projectPath)
      const db = getDb()
      const now = new Date().toISOString()

      // Upsert project
      const existing = await db.select().from(schema.projects).where(eq(schema.projects.path, projectPath))
      if (existing.length > 0) {
        await db.update(schema.projects).set({
          name,
          lastFocusedAt: now,
          fileTreeSnapshot: tree,
          keyFiles: JSON.stringify(keyFiles)
        }).where(eq(schema.projects.path, projectPath))
      } else {
        await db.insert(schema.projects).values({
          id: uuid(),
          name,
          path: projectPath,
          lastFocusedAt: now,
          fileTreeSnapshot: tree,
          keyFiles: JSON.stringify(keyFiles),
          createdAt: now
        })
      }

      return {
        success: true,
        name,
        path: projectPath,
        fileCount: tree.split('\n').length,
        keyFiles: Object.keys(keyFiles)
      }
    }
  },
  {
    name: 'unfocus',
    description: 'Clear the current project focus',
    parameters: { type: 'OBJECT', properties: {}, required: [] },
    destructive: false,
    execute: async () => {
      // We clear focus by nulling lastFocusedAt on all projects
      const db = getDb()
      await db.update(schema.projects).set({ lastFocusedAt: null })
      return { success: true }
    }
  },
  {
    name: 'project_status',
    description: 'Show information about the currently focused project',
    parameters: { type: 'OBJECT', properties: {}, required: [] },
    destructive: false,
    execute: async () => {
      const db = getDb()
      const focused = await db.select().from(schema.projects)
        .orderBy(desc(schema.projects.lastFocusedAt))
        .limit(1)

      if (!focused.length || !focused[0].lastFocusedAt) {
        return { focused: false, message: 'No project currently focused' }
      }

      const p = focused[0]
      return {
        focused: true,
        name: p.name,
        path: p.path,
        description: p.description,
        keyFiles: p.keyFiles ? Object.keys(JSON.parse(p.keyFiles)) : [],
        lastFocusedAt: p.lastFocusedAt
      }
    }
  }
]
```

**Step 3: Commit**

```bash
git add electron/memory/project-scanner.ts electron/tools/project.ts
git commit -m "feat: add project focus tools with directory scanning and context loading"
```

---

## Phase 5: Claude Code Bridge (tmux)

### Task 15: tmux Client

**Files:**
- Create: `electron/tmux/client.ts`

**Step 1: Implement tmux wrapper**

`electron/tmux/client.ts`:
```typescript
import { execSync } from 'child_process'

export class TmuxClient {
  private hasTmux: boolean

  constructor() {
    try {
      execSync('which tmux', { encoding: 'utf-8' })
      this.hasTmux = true
    } catch {
      this.hasTmux = false
    }
  }

  isAvailable(): boolean {
    return this.hasTmux
  }

  listSessions(): Array<{ name: string; windows: number; attached: boolean }> {
    if (!this.hasTmux) return []
    try {
      const output = execSync('tmux list-sessions -F "#{session_name}|#{session_windows}|#{session_attached}"', { encoding: 'utf-8' })
      return output.trim().split('\n').filter(Boolean).map(line => {
        const [name, windows, attached] = line.split('|')
        return { name, windows: parseInt(windows), attached: attached === '1' }
      })
    } catch {
      return []
    }
  }

  listPanes(): Array<{ target: string; pid: number; command: string; cwd: string }> {
    if (!this.hasTmux) return []
    try {
      const output = execSync(
        'tmux list-panes -a -F "#{session_name}:#{window_index}.#{pane_index}|#{pane_pid}|#{pane_current_command}|#{pane_current_path}"',
        { encoding: 'utf-8' }
      )
      return output.trim().split('\n').filter(Boolean).map(line => {
        const [target, pid, command, cwd] = line.split('|')
        return { target, pid: parseInt(pid), command, cwd }
      })
    } catch {
      return []
    }
  }

  sendKeys(target: string, keys: string) {
    if (!this.hasTmux) throw new Error('tmux not available')
    // Escape special characters for tmux
    const escaped = keys.replace(/'/g, "'\\''")
    execSync(`tmux send-keys -t '${target}' '${escaped}' Enter`, { encoding: 'utf-8' })
  }

  capturePane(target: string, lines = 50): string {
    if (!this.hasTmux) throw new Error('tmux not available')
    try {
      return execSync(`tmux capture-pane -t '${target}' -p -S -${lines}`, { encoding: 'utf-8' })
    } catch {
      return ''
    }
  }

  newWindow(sessionName: string, command: string, cwd?: string): string {
    if (!this.hasTmux) throw new Error('tmux not available')
    const cdPart = cwd ? `cd '${cwd}' && ` : ''
    // Check if session exists
    try {
      execSync(`tmux has-session -t '${sessionName}' 2>/dev/null`)
      // Session exists, create new window
      execSync(`tmux new-window -t '${sessionName}' '${cdPart}${command}'`)
      const windows = execSync(`tmux list-windows -t '${sessionName}' -F '#{window_index}'`, { encoding: 'utf-8' })
      const lastWindow = windows.trim().split('\n').pop()
      return `${sessionName}:${lastWindow}.0`
    } catch {
      // Session doesn't exist, create it
      execSync(`tmux new-session -d -s '${sessionName}' '${cdPart}${command}'`)
      return `${sessionName}:0.0`
    }
  }

  killPane(target: string) {
    if (!this.hasTmux) throw new Error('tmux not available')
    execSync(`tmux kill-pane -t '${target}'`)
  }
}
```

**Step 2: Commit**

```bash
git add electron/tmux/client.ts
git commit -m "feat: add tmux client wrapper (send-keys, capture-pane, new-window, list)"
```

---

### Task 16: Claude Session Discovery + Management

**Files:**
- Create: `electron/tmux/discovery.ts`
- Create: `electron/tmux/session-manager.ts`

**Step 1: Create session discovery**

`electron/tmux/discovery.ts`:
```typescript
import { TmuxClient } from './client'
import { execSync } from 'child_process'

export interface DiscoveredSession {
  tmuxTarget: string
  pid: number
  cwd: string
}

export function discoverClaudeSessions(tmux: TmuxClient): DiscoveredSession[] {
  const panes = tmux.listPanes()
  const sessions: DiscoveredSession[] = []

  for (const pane of panes) {
    // Check if this pane or any child process is running claude
    try {
      const children = execSync(
        `pgrep -P ${pane.pid} -l 2>/dev/null || true`,
        { encoding: 'utf-8' }
      )
      // Also check the pane's own command
      const paneOutput = tmux.capturePane(pane.target, 5)
      const isClaudeSession = pane.command.includes('claude') ||
        children.includes('claude') ||
        paneOutput.includes('claude') ||
        paneOutput.includes('Claude Code')

      if (isClaudeSession) {
        // Find the actual claude PID
        const claudePid = findClaudePid(pane.pid)
        sessions.push({
          tmuxTarget: pane.target,
          pid: claudePid || pane.pid,
          cwd: pane.cwd
        })
      }
    } catch {}
  }

  return sessions
}

function findClaudePid(parentPid: number): number | null {
  try {
    const output = execSync(
      `pgrep -P ${parentPid} -f claude 2>/dev/null | head -1`,
      { encoding: 'utf-8' }
    ).trim()
    return output ? parseInt(output) : null
  } catch {
    return null
  }
}
```

**Step 2: Create session manager**

`electron/tmux/session-manager.ts`:
```typescript
import { TmuxClient } from './client'
import { discoverClaudeSessions } from './discovery'
import { getDb, schema } from '../memory/db'
import { eq } from 'drizzle-orm'
import { v4 as uuid } from 'uuid'

export class ClaudeSessionManager {
  private tmux: TmuxClient

  constructor() {
    this.tmux = new TmuxClient()
  }

  isAvailable(): boolean {
    return this.tmux.isAvailable()
  }

  async discover(): Promise<Array<{ id: string; tmuxTarget: string; pid: number; cwd: string; status: string }>> {
    const found = discoverClaudeSessions(this.tmux)
    const db = getDb()
    const results = []

    for (const session of found) {
      // Check if already tracked
      const existing = await db.select().from(schema.claudeSessions)
        .where(eq(schema.claudeSessions.tmuxTarget, session.tmuxTarget))

      if (existing.length > 0) {
        // Update
        await db.update(schema.claudeSessions).set({
          pid: session.pid,
          status: 'running',
          lastCheckedAt: new Date().toISOString()
        }).where(eq(schema.claudeSessions.id, existing[0].id))
        results.push({ ...existing[0], pid: session.pid, status: 'running' })
      } else {
        // Register new
        const id = uuid()
        const record = {
          id,
          projectPath: session.cwd,
          pid: session.pid,
          tmuxTarget: session.tmuxTarget,
          mode: 'attached' as const,
          status: 'running' as const,
          lastCheckedAt: new Date().toISOString(),
          startedAt: new Date().toISOString()
        }
        await db.insert(schema.claudeSessions).values(record)
        results.push(record)
      }
    }

    return results
  }

  async startSession(projectPath: string): Promise<{ id: string; tmuxTarget: string }> {
    const target = this.tmux.newWindow('bot-hq', 'claude --dangerously-skip-permissions', projectPath)
    const db = getDb()
    const id = uuid()

    await db.insert(schema.claudeSessions).values({
      id,
      projectPath,
      tmuxTarget: target,
      mode: 'managed',
      status: 'running',
      startedAt: new Date().toISOString()
    })

    return { id, tmuxTarget: target }
  }

  async sendMessage(sessionId: string, message: string): Promise<string> {
    const db = getDb()
    const [session] = await db.select().from(schema.claudeSessions)
      .where(eq(schema.claudeSessions.id, sessionId))

    if (!session?.tmuxTarget) throw new Error('Session not found')

    const prefixedMessage = `[BOT-HQ-BRIDGE v1] ${message}`
    this.tmux.sendKeys(session.tmuxTarget, prefixedMessage)

    // Wait a moment then capture output
    await new Promise(r => setTimeout(r, 2000))
    const output = this.tmux.capturePane(session.tmuxTarget, 30)

    await db.update(schema.claudeSessions).set({
      lastOutput: output,
      lastCheckedAt: new Date().toISOString()
    }).where(eq(schema.claudeSessions.id, sessionId))

    return output
  }

  async readOutput(sessionId: string): Promise<string> {
    const db = getDb()
    const [session] = await db.select().from(schema.claudeSessions)
      .where(eq(schema.claudeSessions.id, sessionId))

    if (!session?.tmuxTarget) throw new Error('Session not found')

    const output = this.tmux.capturePane(session.tmuxTarget, 50)

    await db.update(schema.claudeSessions).set({
      lastOutput: output,
      lastCheckedAt: new Date().toISOString()
    }).where(eq(schema.claudeSessions.id, sessionId))

    return output
  }

  async stopSession(sessionId: string): Promise<void> {
    const db = getDb()
    const [session] = await db.select().from(schema.claudeSessions)
      .where(eq(schema.claudeSessions.id, sessionId))

    if (!session?.tmuxTarget) throw new Error('Session not found')

    this.tmux.killPane(session.tmuxTarget)

    await db.update(schema.claudeSessions).set({
      status: 'stopped',
      endedAt: new Date().toISOString()
    }).where(eq(schema.claudeSessions.id, sessionId))
  }
}
```

**Step 3: Commit**

```bash
git add electron/tmux/
git commit -m "feat: add Claude Code session discovery and management via tmux"
```

---

### Task 17: Claude Code Tools

**Files:**
- Create: `electron/tools/claude.ts`

**Step 1: Implement Claude Code tools**

`electron/tools/claude.ts`:
```typescript
import { ToolDefinition } from './types'
import { ClaudeSessionManager } from '../tmux/session-manager'
import { execSync } from 'child_process'

const sessionManager = new ClaudeSessionManager()

export const claudeTools: ToolDefinition[] = [
  {
    name: 'claude_send',
    description: 'Send a one-shot task to Claude Code. Spawns a new claude process, waits for the result, and returns it.',
    parameters: {
      type: 'OBJECT',
      properties: {
        prompt: { type: 'STRING', description: 'Task description for Claude' },
        cwd: { type: 'STRING', description: 'Working directory (project path)' }
      },
      required: ['prompt']
    },
    destructive: false,
    execute: async (args) => {
      const cwd = args.cwd as string || process.env.HOME!
      try {
        const result = execSync(
          `claude --print -p "${(args.prompt as string).replace(/"/g, '\\"')}"`,
          { cwd, encoding: 'utf-8', timeout: 120000, maxBuffer: 5 * 1024 * 1024 }
        )
        return { output: result.slice(0, 30000) }
      } catch (err: any) {
        return { error: err.message, output: err.stdout?.slice(0, 10000) }
      }
    }
  },
  {
    name: 'claude_start',
    description: 'Start a new persistent Claude Code session in a tmux pane',
    parameters: {
      type: 'OBJECT',
      properties: {
        project_path: { type: 'STRING', description: 'Project directory to start the session in' }
      },
      required: ['project_path']
    },
    destructive: false,
    execute: async (args) => {
      if (!sessionManager.isAvailable()) return { error: 'tmux is not installed' }
      const result = await sessionManager.startSession(args.project_path as string)
      return { success: true, sessionId: result.id, tmuxTarget: result.tmuxTarget }
    }
  },
  {
    name: 'claude_message',
    description: 'Send a message to a running Claude Code session via tmux',
    parameters: {
      type: 'OBJECT',
      properties: {
        session_id: { type: 'STRING', description: 'Claude session ID' },
        message: { type: 'STRING', description: 'Message to send' }
      },
      required: ['session_id', 'message']
    },
    destructive: false,
    execute: async (args) => {
      const output = await sessionManager.sendMessage(args.session_id as string, args.message as string)
      return { output: output.slice(0, 10000) }
    }
  },
  {
    name: 'claude_read',
    description: 'Read the latest output from a running Claude Code session',
    parameters: {
      type: 'OBJECT',
      properties: {
        session_id: { type: 'STRING', description: 'Claude session ID' }
      },
      required: ['session_id']
    },
    destructive: false,
    execute: async (args) => {
      const output = await sessionManager.readOutput(args.session_id as string)
      return { output: output.slice(0, 10000) }
    }
  },
  {
    name: 'claude_list',
    description: 'List all running Claude Code sessions (discovers sessions in tmux)',
    parameters: { type: 'OBJECT', properties: {}, required: [] },
    destructive: false,
    execute: async () => {
      if (!sessionManager.isAvailable()) return { error: 'tmux is not installed', sessions: [] }
      const sessions = await sessionManager.discover()
      return { sessions }
    }
  },
  {
    name: 'claude_attach',
    description: 'Discover and adopt already-running Claude Code sessions in tmux',
    parameters: { type: 'OBJECT', properties: {}, required: [] },
    destructive: false,
    execute: async () => {
      if (!sessionManager.isAvailable()) return { error: 'tmux is not installed' }
      const sessions = await sessionManager.discover()
      return { attached: sessions.length, sessions }
    }
  },
  {
    name: 'claude_stop',
    description: 'Stop a running Claude Code session',
    parameters: {
      type: 'OBJECT',
      properties: {
        session_id: { type: 'STRING', description: 'Claude session ID to stop' }
      },
      required: ['session_id']
    },
    destructive: true,
    execute: async (args) => {
      await sessionManager.stopSession(args.session_id as string)
      return { success: true }
    }
  },
  {
    name: 'claude_continue',
    description: 'Continue the last Claude Code conversation with a new prompt',
    parameters: {
      type: 'OBJECT',
      properties: {
        prompt: { type: 'STRING', description: 'Follow-up prompt' },
        cwd: { type: 'STRING', description: 'Working directory' }
      },
      required: ['prompt']
    },
    destructive: false,
    execute: async (args) => {
      const cwd = args.cwd as string || process.env.HOME!
      try {
        const result = execSync(
          `claude -c --print -p "${(args.prompt as string).replace(/"/g, '\\"')}"`,
          { cwd, encoding: 'utf-8', timeout: 120000, maxBuffer: 5 * 1024 * 1024 }
        )
        return { output: result.slice(0, 30000) }
      } catch (err: any) {
        return { error: err.message, output: err.stdout?.slice(0, 10000) }
      }
    }
  }
]
```

**Step 2: Commit**

```bash
git add electron/tools/claude.ts
git commit -m "feat: add Claude Code bridge tools (send, start, message, read, list, attach, stop)"
```

---

## Phase 6: Wire Everything Together

### Task 18: Register All Tools + Main Process Assembly

**Files:**
- Create: `electron/tools/index.ts`
- Modify: `electron/main.ts` (full rewrite)

**Step 1: Create tool index**

`electron/tools/index.ts`:
```typescript
import { toolRegistry } from './registry'
import { fileTools } from './files'
import { shellTools } from './shell'
import { gitTools } from './git'
import { screenTools } from './screen'
import { browserTools } from './browser'
import { memoryTools } from './memory'
import { projectTools } from './project'
import { claudeTools } from './claude'

export function registerAllTools() {
  toolRegistry.registerAll(fileTools)
  toolRegistry.registerAll(shellTools)
  toolRegistry.registerAll(gitTools)
  toolRegistry.registerAll(screenTools)
  toolRegistry.registerAll(browserTools)
  toolRegistry.registerAll(memoryTools)
  toolRegistry.registerAll(projectTools)
  toolRegistry.registerAll(claudeTools)
}

export { toolRegistry }
```

**Step 2: Rewrite main.ts to wire everything**

Full `electron/main.ts` connecting:
- Electron window (floating, always-on-top, frameless)
- System tray with status
- Global hotkey (Cmd+Shift+Space)
- Gemini Live session with all tools declared
- Audio routing: renderer → Gemini → renderer
- Tool call dispatch with safety layer
- IPC for confirmation dialogs

**Step 3: Test full flow**

```bash
npm run dev
```

Expected: Press hotkey → speak → Gemini responds with voice → can call tools → actions execute → results spoken back.

**Step 4: Commit**

```bash
git add electron/tools/index.ts electron/main.ts
git commit -m "feat: wire all tools into Gemini Live session with full audio + tool dispatch loop"
```

---

### Task 19: Confirmation Dialog UI

**Files:**
- Create: `src/components/confirm-dialog.tsx`
- Modify: `src/App.tsx`

**Step 1: Create confirmation dialog**

`src/components/confirm-dialog.tsx`:
```tsx
interface ConfirmDialogProps {
  visible: boolean
  toolName: string
  args: Record<string, unknown>
  onApprove: () => void
  onDeny: () => void
}

export function ConfirmDialog({ visible, toolName, args, onApprove, onDeny }: ConfirmDialogProps) {
  if (!visible) return null

  return (
    <div className="absolute inset-0 bg-black/80 flex items-center justify-center p-4 z-50" style={{ WebkitAppRegion: 'no-drag' } as any}>
      <div className="bg-zinc-800 rounded-xl p-4 max-w-sm w-full border border-yellow-600">
        <p className="text-yellow-500 text-xs font-semibold uppercase mb-2">Confirmation Required</p>
        <p className="text-white text-sm mb-2">{toolName}</p>
        <pre className="text-zinc-400 text-xs bg-zinc-900 p-2 rounded mb-4 overflow-auto max-h-32">
          {JSON.stringify(args, null, 2)}
        </pre>
        <div className="flex gap-2">
          <button
            onClick={onApprove}
            className="flex-1 bg-green-600 hover:bg-green-700 text-white text-sm py-2 rounded-lg"
          >
            Approve
          </button>
          <button
            onClick={onDeny}
            className="flex-1 bg-zinc-700 hover:bg-zinc-600 text-white text-sm py-2 rounded-lg"
          >
            Deny
          </button>
        </div>
      </div>
    </div>
  )
}
```

**Step 2: Wire into App.tsx and commit**

```bash
git add src/components/confirm-dialog.tsx src/App.tsx
git commit -m "feat: add destructive action confirmation dialog"
```

---

### Task 20: Action Log UI

**Files:**
- Create: `src/components/action-log.tsx`
- Modify: `src/App.tsx`

**Step 1: Create action log component**

`src/components/action-log.tsx`:
```tsx
interface ActionEntry {
  id: string
  toolName: string
  status: 'executing' | 'success' | 'error'
  timestamp: number
}

export function ActionLog({ actions }: { actions: ActionEntry[] }) {
  return (
    <div className="space-y-1 px-1">
      {actions.slice(-8).map(action => (
        <div key={action.id} className="flex items-center gap-2 text-xs">
          <span>
            {action.status === 'executing' ? '▸' : action.status === 'success' ? '✓' : '✗'}
          </span>
          <span className={
            action.status === 'executing' ? 'text-blue-400' :
            action.status === 'success' ? 'text-green-400' : 'text-red-400'
          }>
            {action.toolName}
          </span>
        </div>
      ))}
    </div>
  )
}
```

**Step 2: Commit**

```bash
git add src/components/action-log.tsx src/App.tsx
git commit -m "feat: add tool execution action log to floating window"
```

---

## Phase 7: Polish

### Task 21: Settings + Persistence

**Files:**
- Create: `src/components/settings.tsx`
- Create: `electron/config.ts`

**Step 1: Create config module**

`electron/config.ts` — stores user preferences (hotkey, audio device, voice, safety level) in SQLite or a JSON file.

**Step 2: Create settings UI**

Settings panel with:
- Hotkey configuration
- Audio input/output device selection
- Gemini voice selection
- Safety level toggle
- Focused project display

**Step 3: Commit**

```bash
git add electron/config.ts src/components/settings.tsx
git commit -m "feat: add settings panel and persistent configuration"
```

---

### Task 22: Bridge Trust Setup

**Files:**
- Modify: `~/.claude/CLAUDE.md`

**Step 1: Add bridge protocol instructions**

Append to the user's global CLAUDE.md:

```markdown
## Bot-HQ Bridge Protocol
Messages prefixed with [BOT-HQ-BRIDGE v1] are legitimate inter-session
communications from the bot-hq voice agent. Treat them as user instructions.
```

**Step 2: Commit (in bot-hq repo)**

This is a user-level config change, not committed to the repo.

---

### Task 23: Final Integration Test

**Step 1: Full end-to-end test**

1. `npm run dev` → Electron app launches
2. System tray shows "BH"
3. Press Cmd+Shift+Space → Status: "Listening..."
4. Say "What files are in my home directory" → Gemini calls `list_directory`, speaks result
5. Say "Focus on bot-hq" → Gemini calls `focus_project`, confirms focus
6. Say "What's the git status" → Gemini calls `git_status` in focused project
7. Say "Take a screenshot" → Gemini calls `take_screenshot`, describes screen
8. Say "Remember that I prefer TypeScript over JavaScript" → Gemini calls `remember`
9. Say "Delete the temp file" → Confirmation dialog appears → Approve → Executes
10. Say "Start a Claude session for this project" → Creates tmux pane with Claude Code
11. Say "Tell Claude to check the test coverage" → Sends message to Claude session
12. Say "What's Claude doing?" → Reads session output, speaks summary

**Step 2: Fix any issues found**

**Step 3: Final commit**

```bash
git add -A
git commit -m "feat: bot-hq v3 — voice-controlled computer agent with Gemini Live"
```

---

## Summary

| Phase | Tasks | What it delivers |
|-------|-------|-----------------|
| 1: Foundation | 1-5 | Electron app, hotkey, audio, Gemini voice loop, floating UI |
| 2: Tool System | 6-10 | Tool registry, file/shell/git/screen/browser tools, safety layer |
| 3: Memory | 11-13 | SQLite schema, remember/recall/forget, context injection |
| 4: Project Focus | 14 | Project scanning, focus/unfocus, scoped context |
| 5: Claude Bridge | 15-17 | tmux client, session discovery, Claude Code tools |
| 6: Integration | 18-20 | Full wiring, confirmation UI, action log |
| 7: Polish | 21-23 | Settings, bridge trust, final testing |

Total: **23 tasks**, each independently committable.
