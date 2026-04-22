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

  const enqueue = useCallback(
    (base64PCM: string) => {
      init()
      const context = contextRef.current!
      const bytes = Uint8Array.from(atob(base64PCM), (c) => c.charCodeAt(0))
      const int16 = new Int16Array(bytes.buffer)
      const float32 = new Float32Array(int16.length)
      for (let i = 0; i < int16.length; i++) {
        float32[i] = int16[i] / 0x7fff
      }
      const buffer = context.createBuffer(1, float32.length, 24000)
      buffer.copyToChannel(float32, 0)
      queueRef.current.push(buffer)
      if (!playingRef.current) playNext()
    },
    [init, playNext]
  )

  const stop = useCallback(() => {
    queueRef.current = []
    playingRef.current = false
  }, [])

  return { enqueue, stop }
}
