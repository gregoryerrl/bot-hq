import { useCallback, useEffect, useState } from 'react'

const VOICE_OPTIONS = ['Kore', 'Puck', 'Charon', 'Fenrir', 'Aoede']

interface SettingsProps {
  visible: boolean
  onClose: () => void
}

export function Settings({ visible, onClose }: SettingsProps) {
  const [voiceName, setVoiceName] = useState('Kore')
  const [alwaysOnTop, setAlwaysOnTop] = useState(true)

  useEffect(() => {
    if (visible) {
      window.api.invoke('config:get').then((cfg: { voiceName: string; alwaysOnTop: boolean }) => {
        setVoiceName(cfg.voiceName)
        setAlwaysOnTop(cfg.alwaysOnTop)
      })
    }
  }, [visible])

  const handleVoiceChange = useCallback((e: React.ChangeEvent<HTMLSelectElement>) => {
    const value = e.target.value
    setVoiceName(value)
    window.api.invoke('config:update', { voiceName: value })
  }, [])

  const handleAlwaysOnTopChange = useCallback(() => {
    const next = !alwaysOnTop
    setAlwaysOnTop(next)
    window.api.invoke('config:update', { alwaysOnTop: next })
  }, [alwaysOnTop])

  if (!visible) return null

  return (
    <div
      className="absolute inset-0 bg-black/80 flex items-center justify-center p-4 z-50"
      style={{ WebkitAppRegion: 'no-drag' } as React.CSSProperties}
    >
      <div className="bg-zinc-800 rounded-xl p-4 max-w-sm w-full border border-zinc-700">
        <div className="flex items-center justify-between mb-4">
          <p className="text-white text-sm font-semibold">Settings</p>
          <button
            onClick={onClose}
            className="text-zinc-400 hover:text-white text-sm"
          >
            Close
          </button>
        </div>

        <div className="space-y-4">
          <div>
            <label className="block text-zinc-400 text-xs mb-1">Voice</label>
            <select
              value={voiceName}
              onChange={handleVoiceChange}
              className="w-full bg-zinc-900 text-white text-sm rounded-lg px-3 py-2 border border-zinc-700 focus:outline-none focus:border-zinc-500"
            >
              {VOICE_OPTIONS.map((v) => (
                <option key={v} value={v}>{v}</option>
              ))}
            </select>
          </div>

          <div className="flex items-center justify-between">
            <label className="text-zinc-400 text-xs">Always on Top</label>
            <button
              onClick={handleAlwaysOnTopChange}
              className={`w-10 h-5 rounded-full transition-colors relative ${
                alwaysOnTop ? 'bg-green-600' : 'bg-zinc-600'
              }`}
            >
              <div
                className={`absolute top-0.5 w-4 h-4 bg-white rounded-full transition-transform ${
                  alwaysOnTop ? 'translate-x-5' : 'translate-x-0.5'
                }`}
              />
            </button>
          </div>
        </div>
      </div>
    </div>
  )
}
