import { Tray, nativeImage, BrowserWindow } from 'electron'

let tray: Tray | null = null

export type TrayStatus = 'idle' | 'listening' | 'thinking' | 'executing' | 'speaking'

const STATUS_LABELS: Record<TrayStatus, string> = {
  idle: 'Bot-HQ — Idle',
  listening: 'Bot-HQ — Listening...',
  thinking: 'Bot-HQ — Thinking...',
  executing: 'Bot-HQ — Executing...',
  speaking: 'Bot-HQ — Speaking...'
}

export function createTray(window: BrowserWindow) {
  const icon = nativeImage.createEmpty()
  tray = new Tray(icon)
  tray.setTitle('BH')
  updateTrayStatus('idle')

  tray.on('click', () => {
    window.isVisible() ? window.hide() : window.show()
  })
}

export function updateTrayStatus(status: TrayStatus) {
  if (!tray) return
  tray.setToolTip(STATUS_LABELS[status])
  tray.setTitle(
    status === 'idle'
      ? 'BH'
      : status === 'listening'
        ? '🎤'
        : status === 'thinking'
          ? '💭'
          : status === 'executing'
            ? '⚡'
            : '🔊'
  )
}
