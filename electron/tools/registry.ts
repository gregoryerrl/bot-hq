import { ToolDefinition } from './types'

class ToolRegistry {
  private tools = new Map<string, ToolDefinition>()

  register(tool: ToolDefinition) {
    this.tools.set(tool.name, tool)
  }

  registerAll(tools: ToolDefinition[]) {
    tools.forEach(t => this.register(t))
  }

  get(name: string): ToolDefinition | undefined {
    return this.tools.get(name)
  }

  getAll(): ToolDefinition[] {
    return Array.from(this.tools.values())
  }

  getFunctionDeclarations() {
    return this.getAll().map(t => ({
      name: t.name,
      description: t.description,
      parameters: t.parameters
    }))
  }
}

export const toolRegistry = new ToolRegistry()
