import { ipcMain, BrowserWindow } from 'electron'

export function setupAudioIPC(window: BrowserWindow) {
  // Renderer sends captured PCM audio chunks (16kHz 16-bit mono)
  ipcMain.on('audio:chunk', (_event, base64PCM: string) => {
    // Will be forwarded to Gemini in Task 4
    window.webContents.send('audio:to-gemini', base64PCM)
  })
}
