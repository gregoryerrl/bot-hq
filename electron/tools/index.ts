import { toolRegistry } from './registry'
import { fileTools } from './files'
import { shellTools } from './shell'
import { gitTools } from './git'
import { screenTools } from './screen'
import { browserTools } from './browser'
import { memoryTools } from './memory'
import { projectTools } from './project'
import { claudeTools } from './claude'

export function registerAllTools() {
  toolRegistry.registerAll(fileTools)
  toolRegistry.registerAll(shellTools)
  toolRegistry.registerAll(gitTools)
  toolRegistry.registerAll(screenTools)
  toolRegistry.registerAll(browserTools)
  toolRegistry.registerAll(memoryTools)
  toolRegistry.registerAll(projectTools)
  toolRegistry.registerAll(claudeTools)
}

export { toolRegistry }
