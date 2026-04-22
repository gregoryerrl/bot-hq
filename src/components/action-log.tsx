export interface ActionEntry {
  id: string
  toolName: string
  status: 'executing' | 'success' | 'error'
  timestamp: number
}

export function ActionLog({ actions }: { actions: ActionEntry[] }) {
  return (
    <div className="space-y-1 px-1">
      {actions.slice(-8).map((action) => (
        <div key={action.id} className="flex items-center gap-2 text-xs">
          <span>
            {action.status === 'executing' ? '▸' : action.status === 'success' ? '✓' : '✗'}
          </span>
          <span
            className={
              action.status === 'executing'
                ? 'text-blue-400'
                : action.status === 'success'
                  ? 'text-green-400'
                  : 'text-red-400'
            }
          >
            {action.toolName}
          </span>
        </div>
      ))}
    </div>
  )
}
