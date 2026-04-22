import { readFileSync, writeFileSync, mkdirSync } from 'fs'
import { join } from 'path'
import { app } from 'electron'

export interface Config {
  hotkey: string
  voiceName: string
  alwaysOnTop: boolean
}

const DEFAULT_CONFIG: Config = {
  hotkey: 'CommandOrControl+Shift+Space',
  voiceName: 'Kore',
  alwaysOnTop: true
}

let config: Config | null = null

function getConfigPath(): string {
  const dir = join(app.getPath('userData'), 'config')
  mkdirSync(dir, { recursive: true })
  return join(dir, 'config.json')
}

export function getConfig(): Config {
  if (config) return config
  try {
    const content = readFileSync(getConfigPath(), 'utf-8')
    config = { ...DEFAULT_CONFIG, ...JSON.parse(content) }
  } catch {
    config = { ...DEFAULT_CONFIG }
  }
  return config!
}

export function updateConfig(updates: Partial<Config>): Config {
  const current = getConfig()
  config = { ...current, ...updates }
  writeFileSync(getConfigPath(), JSON.stringify(config, null, 2))
  return config
}
