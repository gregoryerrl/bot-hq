import type {
  FunctionDeclaration as SDKFunctionDeclaration,
  Type,
} from '@google/genai'
import type { ToolDefinition } from '../tools/types'

export { Type } from '@google/genai'

export interface FunctionDeclaration {
  name: string
  description: string
  parameters: {
    type: Type
    properties: Record<
      string,
      { type: Type; description: string; enum?: string[] }
    >
    required?: string[]
  }
}

/** Convert our FunctionDeclaration to the SDK's format */
export function toSDKFunctionDeclaration(
  decl: FunctionDeclaration
): SDKFunctionDeclaration {
  return {
    name: decl.name,
    description: decl.description,
    parameters: {
      type: decl.parameters.type,
      properties: Object.fromEntries(
        Object.entries(decl.parameters.properties).map(([key, val]) => [
          key,
          { type: val.type, description: val.description, enum: val.enum },
        ])
      ),
      required: decl.parameters.required,
    },
  }
}

/**
 * Convert a ToolDefinition from the tool registry into a FunctionDeclaration for Gemini.
 *
 * The SDK's Type enum uses string values identical to the plain strings in ToolDefinition
 * (e.g. Type.STRING = "STRING", Type.OBJECT = "OBJECT"), so we can cast directly.
 */
export function toolToFunctionDeclaration(
  tool: ToolDefinition
): FunctionDeclaration {
  return {
    name: tool.name,
    description: tool.description,
    parameters: {
      type: (tool.parameters.type || 'OBJECT') as unknown as Type,
      properties: Object.fromEntries(
        Object.entries(tool.parameters.properties).map(([key, val]) => [
          key,
          {
            type: (val.type || 'STRING') as unknown as Type,
            description: val.description,
            enum: val.enum,
          },
        ])
      ),
      required: tool.parameters.required,
    },
  }
}

export interface FunctionCall {
  id: string
  name: string
  args: Record<string, unknown>
}

export type AgentState = 'idle' | 'listening' | 'thinking' | 'executing' | 'speaking'
