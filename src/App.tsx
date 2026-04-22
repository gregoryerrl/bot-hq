export default function App() {
  return (
    <div className="h-screen w-screen rounded-2xl bg-zinc-900 text-white p-4 flex flex-col">
      <div className="flex items-center justify-between mb-4">
        <div className="flex items-center gap-2">
          <div className="w-3 h-3 rounded-full bg-green-500" />
          <span className="text-sm font-semibold">Bot-HQ</span>
        </div>
      </div>
      <div className="flex-1 flex items-center justify-center">
        <p className="text-zinc-500 text-sm">Ready</p>
      </div>
    </div>
  )
}
