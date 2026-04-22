import { useCallback, useEffect, useRef, useState } from 'react'
import { FloatingWindow } from './components/floating-window'
import { Settings } from './components/settings'
import { useAudioCapture } from './hooks/use-audio-capture'
import { useAudioPlayback } from './hooks/use-audio-playback'
import type { ActionEntry } from './components/action-log'

interface Message {
  role: 'user' | 'assistant'
  text: string
}

interface ConfirmRequest {
  id: string
  name: string
  args: Record<string, unknown>
}

export default function App() {
  const [agentState, setAgentState] = useState('idle')
  const [messages, setMessages] = useState<Message[]>([])
  const [focusedProject, setFocusedProject] = useState<string | null>(null)
  const [actions, setActions] = useState<ActionEntry[]>([])
  const [confirmRequest, setConfirmRequest] = useState<ConfirmRequest | null>(null)
  const [settingsVisible, setSettingsVisible] = useState(false)

  const capturingRef = useRef(false)
  const { start: startCapture, stop: stopCapture } = useAudioCapture()
  const { enqueue, stop: stopPlayback } = useAudioPlayback()

  const addMessage = useCallback((role: 'user' | 'assistant', text: string) => {
    setMessages((prev) => [...prev, { role, text }])
  }, [])

  const handleApprove = useCallback(() => {
    if (confirmRequest) {
      window.api.send('tool:confirm-response', { id: confirmRequest.id, approved: true })
      setConfirmRequest(null)
    }
  }, [confirmRequest])

  const handleDeny = useCallback(() => {
    if (confirmRequest) {
      window.api.send('tool:confirm-response', { id: confirmRequest.id, approved: false })
      setConfirmRequest(null)
    }
  }, [confirmRequest])

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

    // Tool confirmation request from ToolDispatcher
    cleanups.push(
      window.api.on('tool:confirm', (data: { id: string; name: string; args: Record<string, unknown> }) => {
        setConfirmRequest(data)
      })
    )

    // Tool started executing
    cleanups.push(
      window.api.on('tool:executing', (data: { name: string; args: Record<string, unknown> }) => {
        const entry: ActionEntry = {
          id: `${data.name}-${Date.now()}`,
          toolName: data.name,
          status: 'executing',
          timestamp: Date.now(),
        }
        setActions((prev) => [...prev, entry])
      })
    )

    // Tool completed
    cleanups.push(
      window.api.on(
        'tool:completed',
        (data: { name: string; success: boolean; duration?: number; error?: string }) => {
          setActions((prev) => {
            // Find the most recent executing entry for this tool and update it
            const idx = [...prev].reverse().findIndex(
              (a) => a.toolName === data.name && a.status === 'executing'
            )
            if (idx === -1) return prev
            const actualIdx = prev.length - 1 - idx
            const updated = [...prev]
            updated[actualIdx] = {
              ...updated[actualIdx],
              status: data.success ? 'success' : 'error',
            }
            return updated
          })
        }
      )
    )

    return () => {
      cleanups.forEach((cleanup) => cleanup())
    }
  }, [addMessage, enqueue, startCapture, stopCapture, stopPlayback])

  return (
    <>
      <FloatingWindow
        state={agentState}
        messages={messages}
        focusedProject={focusedProject}
        actions={actions}
        confirmVisible={confirmRequest !== null}
        confirmToolName={confirmRequest?.name ?? ''}
        confirmArgs={confirmRequest?.args ?? {}}
        onConfirmApprove={handleApprove}
        onConfirmDeny={handleDeny}
        onSettingsOpen={() => setSettingsVisible(true)}
      />
      <Settings
        visible={settingsVisible}
        onClose={() => setSettingsVisible(false)}
      />
    </>
  )
}
