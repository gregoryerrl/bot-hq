import { BrowserWindow } from 'electron'
import { GeminiClient, type GeminiClientConfig } from './client'
import type { FunctionCall, AgentState } from './types'

export type ToolCallHandler = (
  calls: FunctionCall[]
) => Promise<Array<{ id: string; name: string; response: unknown }>>

export type StateChangeHandler = (state: AgentState) => void

export class GeminiSession {
  private client: GeminiClient
  private window: BrowserWindow
  private toolCallHandler: ToolCallHandler | null = null
  private stateChangeHandler: StateChangeHandler | null = null

  constructor(config: GeminiClientConfig, window: BrowserWindow) {
    this.client = new GeminiClient(config)
    this.window = window
    this.setupListeners()
  }

  /**
   * Set a handler for tool calls. When Gemini requests tool execution,
   * this handler is called in the main process. The handler should dispatch
   * tool calls and return results. The session will then send the results
   * back to Gemini automatically.
   */
  setToolCallHandler(handler: ToolCallHandler): void {
    this.toolCallHandler = handler
  }

  /**
   * Set a handler that is called whenever the agent state changes.
   * Useful for updating the tray icon or other status indicators.
   */
  setStateChangeHandler(handler: StateChangeHandler): void {
    this.stateChangeHandler = handler
  }

  private emitState(state: AgentState): void {
    this.window.webContents.send('gemini:state', state)
    if (this.stateChangeHandler) {
      this.stateChangeHandler(state)
    }
  }

  private setupListeners(): void {
    this.client.on('connected', () => {
      this.emitState('idle')
    })

    this.client.on('audio', (base64: string) => {
      this.window.webContents.send('gemini:audio', base64)
      this.emitState('speaking')
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

    this.client.on('tool-call', async (calls: FunctionCall[]) => {
      this.emitState('executing')
      this.window.webContents.send('gemini:tool-calls', calls)

      if (this.toolCallHandler) {
        try {
          const results = await this.toolCallHandler(calls)
          this.sendToolResponse(results)
        } catch (err) {
          console.error('Tool call handler error:', err)
          // Send error responses for all calls so Gemini can continue
          const errorResults = calls.map((call) => ({
            id: call.id,
            name: call.name,
            response: { error: `Tool execution failed: ${(err as Error).message}` },
          }))
          this.sendToolResponse(errorResults)
        }
      }
    })

    this.client.on('turn-complete', () => {
      this.emitState('idle')
      this.window.webContents.send('gemini:turn-complete')
    })

    this.client.on('interrupted', () => {
      this.window.webContents.send('gemini:interrupted')
    })

    this.client.on('error', (error: Error) => {
      this.window.webContents.send('gemini:error', error.message)
    })

    this.client.on('disconnected', () => {
      this.emitState('idle')
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
