import type {
  FunctionDeclaration as SDKFunctionDeclaration,
  Type,
} from '@google/genai'

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

export interface FunctionCall {
  id: string
  name: string
  args: Record<string, unknown>
}

export type AgentState = 'idle' | 'listening' | 'thinking' | 'executing' | 'speaking'
