import { BrowserWindow } from 'electron'
import { GeminiClient, type GeminiClientConfig } from './client'
import type { FunctionCall } from './types'

export class GeminiSession {
  private client: GeminiClient
  private window: BrowserWindow

  constructor(config: GeminiClientConfig, window: BrowserWindow) {
    this.client = new GeminiClient(config)
    this.window = window
    this.setupListeners()
  }

  private setupListeners(): void {
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

  async connect(): Promise<void> {
    await this.client.connect()
  }

  sendAudio(base64PCM: string): void {
    this.client.sendAudio(base64PCM)
  }

  sendToolResponse(
    responses: Array<{ id: string; name: string; response: unknown }>
  ): void {
    this.client.sendToolResponse(responses)
  }

  async disconnect(): Promise<void> {
    await this.client.disconnect()
  }
}
