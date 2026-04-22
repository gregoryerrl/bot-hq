const STATE_CONFIG: Record<string, { label: string; color: string; pulse: boolean }> = {
  idle: { label: 'Ready', color: 'bg-zinc-500', pulse: false },
  listening: { label: 'Listening...', color: 'bg-red-500', pulse: true },
  thinking: { label: 'Thinking...', color: 'bg-yellow-500', pulse: true },
  executing: { label: 'Executing...', color: 'bg-blue-500', pulse: true },
  speaking: { label: 'Speaking...', color: 'bg-green-500', pulse: true },
}

export function StatusIndicator({ state }: { state: string }) {
  const config = STATE_CONFIG[state] || STATE_CONFIG.idle
  return (
    <div className="flex items-center gap-2">
      <div
        className={`w-3 h-3 rounded-full ${config.color} ${config.pulse ? 'animate-pulse' : ''}`}
      />
      <span className="text-sm text-zinc-300">{config.label}</span>
    </div>
  )
}
