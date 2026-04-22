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
      const int16 = new Int16Array(float32.length)
      for (let i = 0; i < float32.length; i++) {
        const s = Math.max(-1, Math.min(1, float32[i]))
        int16[i] = s < 0 ? s * 0x8000 : s * 0x7fff
      }
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
    streamRef.current?.getTracks().forEach((t) => t.stop())
    streamRef.current = null
  }, [])

  return { start, stop }
}
