interface Message {
  role: 'user' | 'assistant'
  text: string
}

export function Transcript({ messages }: { messages: Message[] }) {
  return (
    <div className="flex-1 overflow-y-auto space-y-2 px-1">
      {messages.slice(-10).map((msg, i) => (
        <div
          key={i}
          className={`text-sm ${msg.role === 'user' ? 'text-blue-400' : 'text-zinc-300'}`}
        >
          <span className="text-zinc-600 text-xs">
            {msg.role === 'user' ? 'You' : 'Bot-HQ'}:
          </span>{' '}
          {msg.text}
        </div>
      ))}
    </div>
  )
}
