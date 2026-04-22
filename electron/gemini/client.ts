import { GoogleGenAI, Modality, type LiveServerMessage, type Session } from '@google/genai'
import { EventEmitter } from 'events'
import type { FunctionDeclaration, FunctionCall } from './types'
import { toSDKFunctionDeclaration } from './types'

export interface GeminiClientConfig {
  apiKey: string
  systemInstruction: string
  tools: FunctionDeclaration[]
}

export class GeminiClient extends EventEmitter {
  private ai: GoogleGenAI
  private session: Session | null = null
  private config: GeminiClientConfig

  constructor(config: GeminiClientConfig) {
    super()
    this.config = config
    this.ai = new GoogleGenAI({ apiKey: config.apiKey })
  }

  async connect(): Promise<Session> {
    const sdkDeclarations = this.config.tools.map(toSDKFunctionDeclaration)

    this.session = await this.ai.live.connect({
      model: 'gemini-2.0-flash-live-001',
      config: {
        responseModalities: [Modality.AUDIO],
        systemInstruction: {
          parts: [{ text: this.config.systemInstruction }],
        },
        tools:
          sdkDeclarations.length > 0
            ? [{ functionDeclarations: sdkDeclarations }]
            : undefined,
        speechConfig: {
          voiceConfig: {
            prebuiltVoiceConfig: { voiceName: 'Kore' },
          },
        },
        inputAudioTranscription: {},
        outputAudioTranscription: {},
      },
      callbacks: {
        onopen: () => this.emit('connected'),
        onmessage: (msg: LiveServerMessage) => this.handleMessage(msg),
        onerror: (e: ErrorEvent) => this.emit('error', e),
        onclose: (e: CloseEvent) => this.emit('disconnected', e),
      },
    })

    return this.session
  }

  private handleMessage(msg: LiveServerMessage): void {
    if (msg.toolCall) {
      const calls = (msg.toolCall.functionCalls ?? []).map((fc) => ({
        id: fc.id ?? '',
        name: fc.name ?? '',
        args: fc.args ?? {},
      })) satisfies FunctionCall[]
      this.emit('tool-call', calls)
      return
    }

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

  sendAudio(base64PCM: string): void {
    if (!this.session) return
    this.session.sendRealtimeInput({
      audio: {
        data: base64PCM,
        mimeType: 'audio/pcm;rate=16000',
      },
    })
  }

  sendToolResponse(
    responses: Array<{ id: string; name: string; response: unknown }>
  ): void {
    if (!this.session) return
    this.session.sendToolResponse({
      functionResponses: responses.map((r) => ({
        id: r.id,
        name: r.name,
        response: { result: r.response },
      })),
    })
  }

  async disconnect(): Promise<void> {
    if (this.session) {
      this.session.close()
      this.session = null
    }
  }
}
