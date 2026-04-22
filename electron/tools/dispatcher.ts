import { toolRegistry } from './registry'
import { BrowserWindow, ipcMain } from 'electron'
import { isDestructiveCommand } from '../safety/patterns'

interface DispatchResult {
  id: string
  name: string
  response: unknown
}

export class ToolDispatcher {
  private window: BrowserWindow
  private pendingConfirmations = new Map<string, { resolve: (approved: boolean) => void }>()

  constructor(window: BrowserWindow) {
    this.window = window

    // Listen for confirmation responses from renderer
    ipcMain.on('tool:confirm-response', (_event, data: { id: string; approved: boolean }) => {
      this.handleConfirmationResponse(data.id, data.approved)
    })
  }

  async dispatch(calls: Array<{ id: string; name: string; args: Record<string, unknown> }>): Promise<DispatchResult[]> {
    const results: DispatchResult[] = []

    for (const call of calls) {
      const tool = toolRegistry.get(call.name)
      if (!tool) {
        results.push({ id: call.id, name: call.name, response: { error: `Unknown tool: ${call.name}` } })
        continue
      }

      this.window.webContents.send('tool:executing', { name: call.name, args: call.args })

      const isDestructive = tool.destructive ||
        (call.name === 'run_command' && isDestructiveCommand(call.args.command as string))

      if (isDestructive) {
        const approved = await this.requestConfirmation(call.id, call.name, call.args)
        if (!approved) {
          results.push({ id: call.id, name: call.name, response: { error: 'User denied this action' } })
          continue
        }
      }

      try {
        const startTime = Date.now()
        const result = await tool.execute(call.args)
        const duration = Date.now() - startTime
        this.window.webContents.send('tool:completed', { name: call.name, success: true, duration })
        results.push({ id: call.id, name: call.name, response: result })
      } catch (err: any) {
        this.window.webContents.send('tool:completed', { name: call.name, success: false, error: err.message })
        results.push({ id: call.id, name: call.name, response: { error: err.message } })
      }
    }

    return results
  }

  private requestConfirmation(id: string, name: string, args: Record<string, unknown>): Promise<boolean> {
    return new Promise(resolve => {
      this.pendingConfirmations.set(id, { resolve })
      this.window.webContents.send('tool:confirm', { id, name, args })
    })
  }

  private handleConfirmationResponse(id: string, approved: boolean) {
    const pending = this.pendingConfirmations.get(id)
    if (pending) {
      pending.resolve(approved)
      this.pendingConfirmations.delete(id)
    }
  }
}
