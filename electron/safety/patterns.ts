export const DESTRUCTIVE_PATTERNS = [
  /^rm\s/,
  /^sudo\s/,
  /^kill\s/,
  /^pkill\s/,
  /^shutdown/,
  /^reboot/,
  /^git\s+push.*--force/,
  /^git\s+reset\s+--hard/,
  /^git\s+clean\s+-f/,
  /^chmod\s/,
  /^chown\s/,
  /drop\s+table/i,
  /delete\s+from/i,
  /truncate\s/i,
]

export function isDestructiveCommand(command: string): boolean {
  return DESTRUCTIVE_PATTERNS.some(p => p.test(command.trim()))
}
