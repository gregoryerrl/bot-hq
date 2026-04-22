import { BrowserWindow } from 'electron'

export function setupAudioIPC(_window: BrowserWindow) {
  // Audio routing is now handled in main.ts via GeminiSession.
  // The renderer sends 'audio:chunk' which main.ts forwards to Gemini.
  // Gemini responses are sent back via 'gemini:audio' IPC events from GeminiSession.
  //
  // This module is kept as a no-op placeholder for any future
  // audio processing that may need to happen between capture and Gemini.
}
