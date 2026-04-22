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
    <div
      className="absolute inset-0 bg-black/80 flex items-center justify-center p-4 z-50"
      style={{ WebkitAppRegion: 'no-drag' } as React.CSSProperties}
    >
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
