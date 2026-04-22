import { ActionLog } from './action-log'
import type { ActionEntry } from './action-log'
import { ConfirmDialog } from './confirm-dialog'
import { StatusIndicator } from './status-indicator'
import { Transcript } from './transcript'

interface FloatingWindowProps {
  state: string
  messages: Array<{ role: 'user' | 'assistant'; text: string }>
  focusedProject: string | null
  actions: ActionEntry[]
  confirmVisible: boolean
  confirmToolName: string
  confirmArgs: Record<string, unknown>
  onConfirmApprove: () => void
  onConfirmDeny: () => void
  onSettingsOpen: () => void
}

export function FloatingWindow({
  state,
  messages,
  focusedProject,
  actions,
  confirmVisible,
  confirmToolName,
  confirmArgs,
  onConfirmApprove,
  onConfirmDeny,
  onSettingsOpen,
}: FloatingWindowProps) {
  return (
    <div
      className="relative h-screen w-screen rounded-2xl bg-zinc-900/95 backdrop-blur-sm text-white p-4 flex flex-col border border-zinc-800 select-none"
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
        <div className="flex items-center gap-2">
          <button
            onClick={onSettingsOpen}
            className="text-zinc-500 hover:text-zinc-300 transition-colors"
            style={{ WebkitAppRegion: 'no-drag' } as React.CSSProperties}
            title="Settings"
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z" />
              <circle cx="12" cy="12" r="3" />
            </svg>
          </button>
          <StatusIndicator state={state} />
        </div>
      </div>
      <Transcript messages={messages} />
      {actions.length > 0 && (
        <div className="mt-2 border-t border-zinc-800 pt-2">
          <ActionLog actions={actions} />
        </div>
      )}
      <div className="mt-3 text-center text-zinc-600 text-xs">Cmd+Shift+Space to talk</div>
      <ConfirmDialog
        visible={confirmVisible}
        toolName={confirmToolName}
        args={confirmArgs}
        onApprove={onConfirmApprove}
        onDeny={onConfirmDeny}
      />
    </div>
  )
}
