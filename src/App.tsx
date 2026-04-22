import { useCallback, useEffect, useRef, useState } from 'react'
import { FloatingWindow } from './components/floating-window'
import { useAudioCapture } from './hooks/use-audio-capture'
import { useAudioPlayback } from './hooks/use-audio-playback'

interface Message {
  role: 'user' | 'assistant'
  text: string
}

export default function App() {
  const [agentState, setAgentState] = useState('idle')
  const [messages, setMessages] = useState<Message[]>([])
  const [focusedProject, setFocusedProject] = useState<string | null>(null)

  const capturingRef = useRef(false)
  const { start: startCapture, stop: stopCapture } = useAudioCapture()
  const { enqueue, stop: stopPlayback } = useAudioPlayback()

  const addMessage = useCallback((role: 'user' | 'assistant', text: string) => {
    setMessages((prev) => [...prev, { role, text }])
  }, [])

  useEffect(() => {
    const cleanups: Array<() => void> = []

    // Agent state updates from main process
    cleanups.push(
      window.api.on('gemini:state', (state: string) => {
        setAgentState(state)
      })
    )

    // User transcript from Gemini speech recognition
    cleanups.push(
      window.api.on('gemini:user-transcript', (text: string) => {
        addMessage('user', text)
      })
    )

    // Assistant transcript from Gemini response
    cleanups.push(
      window.api.on('gemini:assistant-transcript', (text: string) => {
        addMessage('assistant', text)
      })
    )

    // Audio chunks from Gemini to play back
    cleanups.push(
      window.api.on('gemini:audio', (base64PCM: string) => {
        enqueue(base64PCM)
      })
    )

    // Focused project changes
    cleanups.push(
      window.api.on('project:focused', (name: string | null) => {
        setFocusedProject(name)
      })
    )

    // Push-to-talk hotkey toggle
    cleanups.push(
      window.api.on('hotkey:push-to-talk', (pressed: boolean) => {
        if (pressed && !capturingRef.current) {
          capturingRef.current = true
          stopPlayback()
          setAgentState('listening')
          startCapture()
        } else if (!pressed && capturingRef.current) {
          capturingRef.current = false
          stopCapture()
          setAgentState('thinking')
          window.api.send('audio:done', null)
        }
      })
    )

    return () => {
      cleanups.forEach((cleanup) => cleanup())
    }
  }, [addMessage, enqueue, startCapture, stopCapture, stopPlayback])

  return <FloatingWindow state={agentState} messages={messages} focusedProject={focusedProject} />
}
