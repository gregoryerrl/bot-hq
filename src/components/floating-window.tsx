import { StatusIndicator } from './status-indicator'
import { Transcript } from './transcript'

interface FloatingWindowProps {
  state: string
  messages: Array<{ role: 'user' | 'assistant'; text: string }>
  focusedProject: string | null
}

export function FloatingWindow({ state, messages, focusedProject }: FloatingWindowProps) {
  return (
    <div
      className="h-screen w-screen rounded-2xl bg-zinc-900/95 backdrop-blur-sm text-white p-4 flex flex-col border border-zinc-800 select-none"
      style={{ WebkitAppRegion: 'drag' } as React.CSSProperties}
    >
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
      <Transcript messages={messages} />
      <div className="mt-3 text-center text-zinc-600 text-xs">Cmd+Shift+Space to talk</div>
    </div>
  )
}
