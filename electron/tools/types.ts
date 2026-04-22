export interface ToolParameter {
  type: string
  description: string
  enum?: string[]
}

export interface ToolDefinition {
  name: string
  description: string
  parameters: {
    type: 'OBJECT'
    properties: Record<string, ToolParameter>
    required?: string[]
  }
  destructive: boolean
  execute: (args: Record<string, unknown>) => Promise<unknown>
}
