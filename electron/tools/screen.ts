import { ToolDefinition } from './types'
import { desktopCapturer, screen as electronScreen } from 'electron'
import { writeFile } from 'fs/promises'
import { join } from 'path'
import { tmpdir } from 'os'
import { v4 as uuid } from 'uuid'

export const screenTools: ToolDefinition[] = [
  {
    name: 'take_screenshot',
    description:
      'Capture a full screenshot of the primary display. Saves a PNG to a temp directory and returns base64 data, file path, and dimensions.',
    parameters: {
      type: 'OBJECT',
      properties: {}
    },
    destructive: false,
    execute: async () => {
      const primaryDisplay = electronScreen.getPrimaryDisplay()
      const { width, height } = primaryDisplay.size
      const scaleFactor = primaryDisplay.scaleFactor

      const sources = await desktopCapturer.getSources({
        types: ['screen'],
        thumbnailSize: {
          width: Math.round(width * scaleFactor),
          height: Math.round(height * scaleFactor)
        }
      })

      const primarySource = sources[0]
      if (!primarySource) {
        return { error: 'No screen source found' }
      }

      const image = primarySource.thumbnail
      if (image.isEmpty()) {
        return { error: 'Captured image is empty' }
      }

      const pngBuffer = image.toPNG()
      const base64 = pngBuffer.toString('base64')

      const filename = `screenshot-${uuid()}.png`
      const filePath = join(tmpdir(), filename)
      await writeFile(filePath, pngBuffer)

      return {
        base64,
        path: filePath,
        width: image.getSize().width,
        height: image.getSize().height
      }
    }
  },

  {
    name: 'read_screen',
    description:
      'Take a screenshot for Gemini to analyze. Optionally provide a question about what is on screen. Returns base64 image data and dimensions.',
    parameters: {
      type: 'OBJECT',
      properties: {
        question: {
          type: 'STRING',
          description: 'Optional question about what is on screen for Gemini to answer'
        }
      }
    },
    destructive: false,
    execute: async () => {
      const primaryDisplay = electronScreen.getPrimaryDisplay()
      const { width, height } = primaryDisplay.size
      const scaleFactor = primaryDisplay.scaleFactor

      const sources = await desktopCapturer.getSources({
        types: ['screen'],
        thumbnailSize: {
          width: Math.round(width * scaleFactor),
          height: Math.round(height * scaleFactor)
        }
      })

      const primarySource = sources[0]
      if (!primarySource) {
        return { error: 'No screen source found' }
      }

      const image = primarySource.thumbnail
      if (image.isEmpty()) {
        return { error: 'Captured image is empty' }
      }

      const pngBuffer = image.toPNG()
      const base64 = pngBuffer.toString('base64')

      return {
        base64,
        width: image.getSize().width,
        height: image.getSize().height,
        mimeType: 'image/png'
      }
    }
  }
]
