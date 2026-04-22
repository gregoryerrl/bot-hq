import { isDestructiveCommand } from './patterns'

export function checkSafety(toolName: string, args: Record<string, unknown>): boolean {
  if (toolName === 'run_command') {
    return isDestructiveCommand(args.command as string)
  }
  return false
}
